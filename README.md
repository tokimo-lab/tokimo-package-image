# tokimo-package-image

⚠️ **This crate no longer builds the libvips native binaries itself. Native builds (Linux/macOS/Windows) live in [tokimo-lab/tokimo-lib](https://github.com/tokimo-lab/tokimo-lib). This repo only contains the safe Rust FFI bindings consumed via libloading.**

On-the-fly image resize, thumbnail generation, RAW preview extraction & EXIF parsing for Tokimo.

## Features

- **`ThumbnailGenerator`** — concurrency-controlled async thumbnail generation, capped at half the available CPU cores so the tokio runtime keeps threads for I/O work
- **libvips-backed resizing** — fast, low-memory pipeline encoding to WebP / PNG / JPEG (`OutputFormat`) for common raster formats
- **HEIC/HEIF tile-grid decoding** — routes HEIC/HEIF through `tokimo-package-ffmpeg` before libvips fallback so tiled iOS images are assembled before resize
- **RAW preview extraction** — pulls embedded JPEG previews from CR2 / NEF / ARW / DNG / RAF / RW2 / ORF and other common RAW formats
- **EXIF reading** — `extract_exif` (from path) and `extract_exif_from_bytes` returning structured `ExifData` (camera, lens, dates, GPS, exposure)
- **Date / dimension metadata helpers** — `get_image_dimensions`, `get_image_dimensions_from_bytes`, `extract_date_from_filename`, `file_mtime_as_date`

## Usage

```rust
use std::sync::Arc;
use tokimo_package_image::{OutputFormat, ThumbnailGenerator};

let generator = Arc::new(ThumbnailGenerator::new());

// Generate a 320x320 WebP thumbnail.
let (bytes, content_type) = generator
    .generate("/media/photo.jpg", 320, 320, OutputFormat::Webp)
    .await?;

assert_eq!(content_type, "image/webp");
println!("encoded {} bytes", bytes.len());
```

EXIF extraction:

```rust
use tokimo_package_image::extract_exif;

let exif = extract_exif("/media/photo.jpg")?;
println!("{:?} @ {:?}", exif.camera_model, exif.date_taken);
```

## Cargo

```toml
tokimo-package-image = { git = "https://github.com/tokimo-lab/tokimo-package-image" }
```

## License

MIT

## Native dependencies

This crate links **libvips** at compile time (via `#[link(...)]` declarations
in `src/vips.rs`) and uses `tokimo-package-ffmpeg` for HEIC/HEIF tile-grid
assembly before resize. Inside the [`tokimo.io`](https://github.com/tokimo-lab/tokimo.io)
monorepo these native dependencies are resolved automatically: libvips is populated
by `pnpm deps --dep tokimo-lib` into `bin/tokimo-lib/current`. Outside the monorepo
you need to provide them yourself.

Easiest path:

```bash
# Pick your platform
PLATFORM=linux  # or macos-arm64 / windows

# libvips (compile + runtime)
mkdir -p .libvips-install
cd .libvips-install
gh release download v8.18.2-tokimo.3 -R tokimo-lab/tokimo-package-libvips \
  -p install-${PLATFORM}.tar.zst
tar --zstd -xf install-${PLATFORM}.tar.zst
cd ..
export TOKIMO_DEP_LIBVIPS_DIR=$PWD/.libvips-install/install

# Runtime loader path
# Linux:
export LD_LIBRARY_PATH=$TOKIMO_DEP_LIBVIPS_DIR/lib
# macOS:
# export DYLD_FALLBACK_LIBRARY_PATH=$TOKIMO_DEP_LIBVIPS_DIR/lib
# Windows (PowerShell):
# $env:PATH = "$env:TOKIMO_DEP_LIBVIPS_DIR\bin;" + $env:PATH

cargo build
```

On macOS you'll additionally need the brew runtime dependencies that the
prebuilt libvips dylibs were linked against — `brew install vips` is enough.
Inside the tokimo.io monorepo none of this is needed: libvips is resolved via
`pnpm deps`.
