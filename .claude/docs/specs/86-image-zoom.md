# 86. Picture zoom in Lister

ROADMAP «Lister, дальше: масштаб картинок». Builds on 85 (the same keys).

- A raster picture's size is read from its header when the file loads (`image::ImageReader`
  `into_dimensions`, no full decode) into `Image::Raster(handle, size)`.
- `Lister.zoom: Option<f32>`; `None` = fitted (ScaleDown, as before). `+` / `-` / Num± in image
  mode: `core::lister::zoom_step` from the shown scale (`fit_scale` at first) over `ZOOMS`
  (10 %…800 %). Zoomed: the image at its size × zoom; bigger than the window — a scrollable in
  both directions, else centred.
- The info line: «W×H, N%, size». A «Вписать» button while zoomed sets `zoom = None`; a new
  picture (N / P) opens fitted.
- SVG has no pixel size: stays fitted, the keys do nothing. While the file is still being read the
  keys do nothing (its mode is not known yet).
