# Changelog for `fdsm`

## 0.8.0 (2025-10-02)

* Update `nalgebra` dependency to 0.34.1

## 0.7.0 (2025-05-23)

* Move `ttf-parser` integration to a separate crate

## 0.6.0 (2024-09-13)

* Update `ttf-parser` dependency (for the feature with the same name)
* Speed up error correction by yeeting all the texture bounds checks
  * I’m serious. This reduces the time required for error correction by about 20%.
* Speed up MSDF generation:
  * Unprepare segments a bit less eagerly in `PreparedComponent::distance`
  * Optimize `solve_cubic_normed` a bit

## 0.5.0 (2024-06-12)

* Update `ttf-parser` dependency (for the feature with the same name)

## 0.4.0 (2024-05-06)

* Update dependencies
* Add missing docs

## 0.3.0 (2023-10-18)

* Correct pixel value calculation in render methods
* Add support for error correction
  * Only the standard error correction algorithm (equivalent to msdfgen’s `msdfErrorCorrection`) is currently supported, not the shapeless or the legacy algorithms
  * Overlap support setting is not supported

## 0.2.0 (2023-10-15)

Bump optional `ttf-parser` dependency to 0.20.0.

## 0.1.2 (2023-09-06)

Remove stray debug print.

## 0.1.1 (2023-08-26)

Fix bug where `correct_sign_mtsdf` did not correct the sign of the alpha component.

## 0.1.0 (2023-08-26)

Initial release.
