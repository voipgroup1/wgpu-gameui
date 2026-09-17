//! A per-frame arena of uniform slots, bound with dynamic offsets.
//!
//! Three renderers here (the UI pipelines, the MSDF text/icon pipeline, and the
//! backdrop blur) record several passes into one `CommandEncoder` that is submitted
//! once. They cannot share a uniform buffer:
//!
//! `Queue::write_buffer` does **not** take effect when it is called. It is staged and
//! executed at the start of the next `submit`, i.e. before any recorded pass runs. Two
//! passes that write the same byte range therefore both read whichever write landed
//! last — the later pass's — and the earlier pass silently renders with data meant for
//! another pass, at another size, with another tint. That is not a subtle hazard: with
//! two `UiRenderer::render` calls in one submission the first pass's geometry vanishes
//! entirely.
//!
//! Handing every pass its own slot is what keeps them independent. Slots are handed out
//! in `min_uniform_buffer_offset_alignment` strides (the alignment a dynamic offset must
//! satisfy) and [`UniformArena::reset`] — called once per frame, from
//! `UiRenderer::begin_frame` — makes them reusable, so the arena is one buffer whose
//! size tracks the frame's high-water mark rather than an allocation per pass.
//!
//! Growth replaces the buffer. The bind group names the buffer, so it is rebuilt too,
//! and the replaced buffer is retained: the encoder still references it, and wgpu keeps
//! resources referenced by recorded commands alive until they complete.

/// Align `offset` up to `alignment`. `alignment` is always a power of two here (it
/// comes from the adapter's `min_uniform_buffer_offset_alignment` limit), but the
/// arithmetic does not depend on that.
pub(crate) fn aligned_slot(offset: u64, alignment: u64) -> u64 {
    let alignment = alignment.max(1);
    offset.div_ceil(alignment) * alignment
}

pub(crate) struct UniformArena {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    label: &'static str,
    /// Size of one uniform struct.
    slot_size: u64,
    /// Stride between slots: the adapter's uniform-offset alignment.
    alignment: u64,
    capacity: u64,
    /// Bytes handed out this frame.
    offset: u64,
    /// Buffers replaced by growth, retained because recorded commands reference them.
    retired: Vec<wgpu::Buffer>,
}

impl UniformArena {
    /// Create an arena whose slots hold `slot_size` bytes each, with `visibility`
    /// declaring which shader stages read the uniform.
    pub(crate) fn new(
        device: &wgpu::Device,
        label: &'static str,
        slot_size: u64,
        visibility: wgpu::ShaderStages,
    ) -> Self {
        let alignment = (device.limits().min_uniform_buffer_offset_alignment as u64).max(slot_size);
        // Start at four slots: enough for the common frame, small enough to not care.
        let capacity = alignment * 4;
        let buffer = Self::make_buffer(device, label, capacity);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(slot_size),
                },
                count: None,
            }],
        });
        let bind_group = Self::make_bind_group(device, &layout, &buffer, slot_size);
        Self {
            buffer,
            bind_group,
            layout,
            label,
            slot_size,
            alignment,
            capacity,
            offset: 0,
            retired: Vec::new(),
        }
    }

    /// Start a frame: the next [`allocate`](Self::allocate) hands out the first slot
    /// again. Call once per frame, not once per pass.
    pub(crate) fn reset(&mut self) {
        self.offset = 0;
    }

    /// Reserve one slot and return its byte offset — the dynamic offset to bind with
    /// [`bind_group`](Self::bind_group). The second value reports whether the arena had
    /// to grow, so the caller can count the reallocation.
    pub(crate) fn allocate(&mut self, device: &wgpu::Device) -> (u64, bool) {
        let slot = aligned_slot(self.offset, self.alignment);
        let needed = slot + self.slot_size;
        let mut grew = false;
        if needed > self.capacity {
            grew = true;
            self.capacity = needed.next_power_of_two();
            let replacement = Self::make_buffer(device, self.label, self.capacity);
            self.bind_group =
                Self::make_bind_group(device, &self.layout, &replacement, self.slot_size);
            self.retired
                .push(std::mem::replace(&mut self.buffer, replacement));
        }
        self.offset = needed;
        (slot, grew)
    }

    /// The buffer to write `queue.write_buffer(..)` into, at the offset
    /// [`allocate`](Self::allocate) returned.
    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// The bind group to bind at group 0, with the slot offset as its dynamic offset.
    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// The arena's bind-group layout, for pipelines that bind it.
    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Bytes handed out since the last [`reset`](Self::reset) — part of the frame's
    /// GPU-arena footprint.
    pub(crate) fn bytes_used(&self) -> u64 {
        self.offset
    }

    fn make_buffer(device: &wgpu::Device, label: &'static str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        slot_size: u64,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform arena bg"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(slot_size),
                }),
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::aligned_slot;

    #[test]
    fn slots_start_at_zero_and_advance_by_the_alignment() {
        assert_eq!(aligned_slot(0, 256), 0);
        // A 64-byte uniform still takes a 256-byte slot: dynamic offsets may only
        // be multiples of the adapter's alignment.
        assert_eq!(aligned_slot(64, 256), 256);
        assert_eq!(aligned_slot(256, 256), 256);
        assert_eq!(aligned_slot(257, 256), 512);
    }

    #[test]
    fn alignment_of_one_is_a_plain_bump() {
        assert_eq!(aligned_slot(0, 1), 0);
        assert_eq!(aligned_slot(64, 1), 64);
        assert_eq!(aligned_slot(65, 1), 65);
    }

    #[test]
    fn a_zero_alignment_does_not_divide_by_zero() {
        assert_eq!(aligned_slot(0, 0), 0);
        assert_eq!(aligned_slot(64, 0), 64);
    }
}
