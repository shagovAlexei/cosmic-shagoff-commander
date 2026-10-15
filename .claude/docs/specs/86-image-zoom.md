# 86. Picture zoom in Lister

ROADMAP «Lister, дальше: масштаб картинок». Builds on 85 (the same keys).

- A raster picture's size is read from its header when the file loads (`image::ImageReader` →
  decoder `dimensions`, no full decode; swapped for an EXIF orientation of 90° / 270°, as the
  renderer turns the picture) into `Image::Raster(handle, size)`.
- The image view sits in a size sensor (`ListerResized`): the fit scale is reckoned from the real
  window, not the text view's last size.
- `Lister.zoom: Option<f32>`; `None` = fitted (ScaleDown, as before). `+` / `-` / Num± in image
  mode: `core::lister::zoom_step` from the shown scale (`fit_scale` at first) over `ZOOMS`
  (10 %…800 %). Zoomed: the image at its size × zoom; bigger than the window — a scrollable in
  both directions (the viewer's scroll id: ↑ ↓ ← → by 40 px, PgUp / PgDn by the window, Home /
  End), else centred. A step keeps the middle of the view on the same spot (from fitted: the
  picture's middle).
- Not done: HiDPI — 100 % is the picture's pixels in logical units (×2 device pixels at 200 %).
- The info line: «W×H, N%, size». A «Вписать» button while zoomed sets `zoom = None`; a new
  picture (N / P) opens fitted.
- SVG has no pixel size: stays fitted, the keys do nothing. While the file is still being read the
  keys do nothing (its mode is not known yet).
