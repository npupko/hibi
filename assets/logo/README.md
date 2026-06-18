# Hibi — brand assets

The mark is the lowercase wordmark **`hibi`** with a cinnabar seal carrying the two kanji **日々**
(*hibi*, "day after day"). Generated with Nano Banana Pro and post-processed deterministically.

## Palette

| Role | Hex | Notes |
|------|-----|-------|
| Cinnabar (朱) | `#D6452F` | the seal, the dots on the `i`s |
| Sumi ink | `#1B1B1A` | the wordmark on light |
| Rice-paper | `#F3EDE1` | light background |
| Cream | `#F0E8D5` | the 日々 inside the seal / wordmark on dark |

## Assets

| File | Size | Use |
|------|------|-----|
| `hibi-wordmark.png` | 2048² | primary wordmark, light/paper background |
| `hibi-wordmark-dark.png` | 2752×1536 | wordmark for dark backgrounds |
| `hibi-wordmark-transparent.png` | 1955×652 | wordmark with **real alpha** — drop on any background |
| `hibi-mark.png` | 2048² | square mark (seal only) — avatars, social |
| `favicon-16/32/48.png` | 16/32/48 | browser favicons (PNG) |
| `favicon.ico` | multi | legacy `.ico` (16+32+48) |
| `apple-touch-icon.png` | 180² | iOS home-screen icon |
| `icon-192.png`, `icon-512.png` | 192/512 | PWA / Android manifest icons |
| `hibi-og.png` | 1200×630 | Open Graph / social share card |

## Usage

**README header** (auto dark-mode):

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo/hibi-wordmark-dark.png">
  <img alt="Hibi 日々" src="assets/logo/hibi-wordmark-transparent.png" width="300">
</picture>
```

**Web `<head>`:**

```html
<link rel="icon" type="image/png" sizes="32x32" href="/favicon-32.png">
<link rel="icon" href="/favicon.ico" sizes="any">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<meta property="og:image" content="/hibi-og.png">
```

## Regenerating / transparency

PNGs are raster (Nano Banana Pro has no native alpha). The transparent wordmark is produced by
generating on a **chroma-key green** screen, then keying with ImageMagick (HSV removal + green-channel
de-spill + trim). Favicons are downscaled from `hibi-mark.png`. A clean vector (SVG) redraw is the
recommended next step if infinitely-scalable output is needed.
