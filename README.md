# tokimo-package-image

On-the-fly image resize, thumbnail generation (libvips), RAW preview extraction & EXIF parsing for Tokimo.

## Features

- **`ThumbnailGenerator`** — concurrency-controlled async thumbnail generation, capped at half the available CPU cores so the tokio runtime keeps threads for I/O work
- **libvips-backed resizing** — fast, low-memory pipeline encoding to WebP / PNG / JPEG (`OutputFormat`)
- **RAW preview extraction** — pulls embedded JPEG previews from CR2 / NEF / ARW / DNG / RAF / RW2 / ORF and other common RAW formats
- **EXIF reading** — `extract_exif` (from path) and `extract_exif_from_bytes` returning structured `ExifData` (camera, lens, dates, GPS, exposure)
- **Date / dimension metadata helpers** — `get_image_dimensions`, `get_image_dimensions_from_bytes`, `extract_date_from_filename`, `extract_date_via_ffprobe`, `file_mtime_as_date`, `get_dimensions_via_ffprobe`
- **ffmpeg fallback** — for exotic formats libvips can't decode (HEIC/HEIF variants, obscure containers), falls back to an `ffmpeg` binary if a path is supplied

## Usage

```rust
use std::sync::Arc;
use tokimo_package_image::{OutputFormat, ThumbnailGenerator};

let generator = Arc::new(ThumbnailGenerator::new());

// Generate a 320x320 WebP thumbnail; pass Some(ffmpeg_path) to enable
// the ffmpeg fallback for formats libvips can't decode.
let (bytes, content_type) = generator
    .generate("/media/photo.jpg", 320, 320, OutputFormat::Webp, None)
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
in `src/vips.rs`) and uses **ffmpeg** as an optional runtime fallback. Inside
the [`tokimo.io`](https://github.com/tokimo-lab/tokimo.io) monorepo both are
resolved automatically — libvips via `bin/libvips/current` populated by
`pnpm deps --dep libvips`, ffmpeg via a `[patch]` redirect to the local
`tokimo-package-ffmpeg` checkout. Outside the monorepo you need to provide
both yourself.

Easiest path:

```bash
# Pick your platform
PLATFORM=linux  # or macos-arm64 / windows

# 1. libvips (compile + runtime)
mkdir -p .libvips-install
cd .libvips-install
gh release download v8.18.2-tokimo.3 -R tokimo-lab/tokimo-package-libvips \
  -p install-${PLATFORM}.tar.zst
tar --zstd -xf install-${PLATFORM}.tar.zst
cd ..
export TOKIMO_DEP_LIBVIPS_DIR=$PWD/.libvips-install/install

# 2. ffmpeg (transitive build dep via tokimo-package-ffmpeg)
mkdir -p .ffmpeg-install
cd .ffmpeg-install
gh release download nightly -R tokimo-lab/tokimo-package-ffmpeg \
  -p install-${PLATFORM}.tar.zst
tar --zstd -xf install-${PLATFORM}.tar.zst
cd ..
export FFMPEG_PKG_CONFIG_PATH=$PWD/.ffmpeg-install/install/lib/pkgconfig
export FFMPEG_INCLUDE_DIR=$PWD/.ffmpeg-install/install/include
export FFMPEG_DYN_DIR=$PWD/.ffmpeg-install/install/lib

# 3. Runtime loader path (combine both)
# Linux:
export LD_LIBRARY_PATH=$TOKIMO_DEP_LIBVIPS_DIR/lib:$FFMPEG_DYN_DIR
# macOS:
# export DYLD_FALLBACK_LIBRARY_PATH=$TOKIMO_DEP_LIBVIPS_DIR/lib:$FFMPEG_DYN_DIR
# Windows (PowerShell):
# $env:PATH = "$env:TOKIMO_DEP_LIBVIPS_DIR\bin;$env:FFMPEG_DYN_DIR;" + $env:PATH

cargo build
```

On macOS you'll additionally need the brew runtime dependencies that the
prebuilt ffmpeg + libvips dylibs were linked against — `brew install vips`
plus the ffmpeg deps listed in `.github/workflows/ci.yml`. Inside the
tokimo.io monorepo none of this is needed: the workspace builds
`tokimo-package-ffmpeg` from local source and resolves libvips via
`pnpm deps`.
