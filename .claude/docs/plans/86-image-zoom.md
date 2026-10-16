# 86. Picture zoom — plan

Spec: `../specs/86-image-zoom.md`.

1. Test first in core: `fit_scale`, `zoom_step` (from a step, from a fit scale, the ends).
2. `Image::Raster` gets the header size; `Lister.zoom`, `image_size`, `zoom_image`; reset in
   `set_loaded`.
3. View: zoomed image fixed size, scrollable both ways when bigger; info with size and percent;
   «Вписать» → `Message::ListerFit`.
4. `App::lister_zoom`: image mode (or still reading) → `zoom_image`, never the text size.
5. Tests: a generated 2000×1000 BMP; headless on a COSMIC wallpaper.
