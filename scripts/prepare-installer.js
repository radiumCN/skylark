/**
 * Generates installer BMP assets required by NSIS from SVG templates.
 * Runs automatically as part of `npm run build` via beforeBuildCommand.
 *
 * NSIS requirements:
 *   sidebarImage  → 164 × 314 px  24-bit BMP  (Welcome / Finish page left panel)
 *   headerImage   → 150 × 57  px  24-bit BMP  (inner pages top banner)
 *
 * Using SVG → PNG → BMP pipeline ensures the exact portrait/landscape aspect
 * ratios are preserved without distortion (AI-generated PNGs are landscape).
 *
 * macOS DMG background stays as PNG (Tauri accepts it natively).
 *
 * Every logo mark here is derived from `src-tauri/icons/icon.png` (the single
 * source of truth for the app icon) rather than hand-drawn, so a future icon
 * change can never leave the tray / installer showing a stale logo.
 */

import sharp from "sharp";
import { writeFileSync, mkdirSync } from "fs";
import { dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = `${__dirname}/..`;

/** Single source of truth for the app icon. */
const APP_ICON = `${root}/src-tauri/icons/icon.png`;

/** SVGs are rasterized at 2× (144 dpi vs the 72 dpi default) then downscaled. */
const SCALE = 2;

// ─── SVG Designs ──────────────────────────────────────────────────────────────

/** Sidebar: 164 × 314 px — Welcome / Finish page left panel */
const SIDEBAR_SVG = `
<svg width="164" height="314" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%"   stop-color="#0f172a"/>
      <stop offset="100%" stop-color="#0c1833"/>
    </linearGradient>
    <linearGradient id="glow" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%"   stop-color="#3b82f6" stop-opacity="0.25"/>
      <stop offset="100%" stop-color="#3b82f6" stop-opacity="0"/>
    </linearGradient>
  </defs>

  <!-- Background -->
  <rect width="164" height="314" fill="url(#bg)"/>

  <!-- Top decorative glow -->
  <ellipse cx="82" cy="0" rx="110" ry="60" fill="url(#glow)"/>

  <!-- Network mesh — dots -->
  <circle cx="20"  cy="18"  r="1.5" fill="#1e3a5f" opacity="0.9"/>
  <circle cx="55"  cy="8"   r="1.5" fill="#1e3a5f" opacity="0.9"/>
  <circle cx="90"  cy="22"  r="1.5" fill="#2563eb" opacity="0.8"/>
  <circle cx="130" cy="10"  r="1.5" fill="#1e3a5f" opacity="0.9"/>
  <circle cx="148" cy="35"  r="1.5" fill="#1e3a5f" opacity="0.7"/>
  <circle cx="8"   cy="48"  r="1.5" fill="#1e3a5f" opacity="0.7"/>
  <circle cx="42"  cy="40"  r="1.5" fill="#2563eb" opacity="0.8"/>
  <circle cx="115" cy="45"  r="1.5" fill="#1e3a5f" opacity="0.9"/>

  <!-- Network mesh — connecting lines -->
  <line x1="20"  y1="18"  x2="55"  y2="8"   stroke="#1e3a5f" stroke-width="0.8" opacity="0.7"/>
  <line x1="55"  y1="8"   x2="90"  y2="22"  stroke="#2563eb" stroke-width="0.8" opacity="0.6"/>
  <line x1="90"  y1="22"  x2="130" y2="10"  stroke="#1e3a5f" stroke-width="0.8" opacity="0.7"/>
  <line x1="130" y1="10"  x2="148" y2="35"  stroke="#1e3a5f" stroke-width="0.8" opacity="0.6"/>
  <line x1="20"  y1="18"  x2="8"   y2="48"  stroke="#1e3a5f" stroke-width="0.8" opacity="0.6"/>
  <line x1="8"   y1="48"  x2="42"  y2="40"  stroke="#2563eb" stroke-width="0.8" opacity="0.6"/>
  <line x1="42"  y1="40"  x2="90"  y2="22"  stroke="#1e3a5f" stroke-width="0.8" opacity="0.5"/>
  <line x1="115" y1="45"  x2="148" y2="35"  stroke="#1e3a5f" stroke-width="0.8" opacity="0.6"/>
  <line x1="55"  y1="8"   x2="42"  y2="40"  stroke="#1e3a5f" stroke-width="0.8" opacity="0.5"/>

  <!-- App icon is composited here at (50,66) 64×64 — see SIDEBAR_OVERLAY -->

  <!-- Skylark title -->
  <text x="82" y="155"
        font-family="Arial,sans-serif" font-size="19" font-weight="bold"
        text-anchor="middle" fill="white" letter-spacing="0.5">Skylark</text>

  <!-- Subtitle -->
  <text x="82" y="172"
        font-family="Arial,sans-serif" font-size="9.5"
        text-anchor="middle" fill="#94a3b8" letter-spacing="1">云雀 · 代理客户端</text>

  <!-- Divider -->
  <line x1="32" y1="272" x2="132" y2="272" stroke="#1e3a5f" stroke-width="1"/>

  <!-- Branding -->
  <text x="82" y="290"
        font-family="Arial,sans-serif" font-size="9"
        text-anchor="middle" fill="#475569">by Radium</text>
</svg>
`;

/** Header: 150 × 57 px — inner pages top banner */
const HEADER_SVG = `
<svg width="150" height="57" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="hbg" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%"   stop-color="#0f172a"/>
      <stop offset="80%"  stop-color="#0f1f40"/>
      <stop offset="100%" stop-color="#162040"/>
    </linearGradient>
  </defs>

  <!-- Background -->
  <rect width="150" height="57" fill="url(#hbg)"/>

  <!-- Right-side accent diagonal -->
  <line x1="118" y1="0" x2="150" y2="57" stroke="#2563eb" stroke-width="1.2" opacity="0.5"/>
  <line x1="130" y1="0" x2="150" y2="36" stroke="#3b82f6" stroke-width="0.7" opacity="0.35"/>

  <!-- App icon is composited here at (12,14) 30×30 — see HEADER_OVERLAY -->

  <!-- Product name -->
  <text x="48" y="27"
        font-family="Arial,sans-serif" font-size="14" font-weight="bold"
        fill="white" letter-spacing="0.3">Skylark</text>

  <!-- Subtitle -->
  <text x="48" y="41"
        font-family="Arial,sans-serif" font-size="8.5"
        fill="#94a3b8" letter-spacing="0.8">跨平台代理客户端</text>
</svg>
`;

/**
 * macOS menu-bar tray icon — TEMPLATE image, derived from the app icon.
 *
 * A template image carries no colour: macOS reads only its alpha channel and
 * paints the opaque pixels in the menu-bar's text colour (black on a light bar,
 * white on a dark one). So we turn the app icon's *white lark glyph* into the
 * alpha channel and throw the blue plate away — feeding the colour icon in
 * directly makes it invisible on a dark menu bar.
 *
 * Pipeline: crop to the glyph's bounding box → map luminance to alpha (the icon
 * is cleanly bimodal, ~65 for the blue plate vs ~245 for the glyph) → downscale.
 *
 * tray-icon scales whatever we hand it to an 18 pt height, so the canvas size
 * only sets the resolution ceiling: 88 px keeps it crisp on a 2× Retina bar.
 */
const TRAY_PX = 88;

/** Luminance window mapped onto 0…255 alpha. Below LO → transparent, above HI → solid. */
const GLYPH_LUMA_LO = 95;
const GLYPH_LUMA_HI = 235;

/** A pixel this bright is considered part of the glyph when measuring its bounds. */
const GLYPH_BBOX_LUMA = 170;

const luma = (r, g, b) => 0.299 * r + 0.587 * g + 0.114 * b;

/**
 * Render the app icon's glyph as an alpha-only black mask.
 * @returns {Promise<Buffer>} PNG bytes, TRAY_PX × TRAY_PX
 */
async function renderTrayTemplate() {
  const { data, info } = await sharp(APP_ICON)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const { width: w, height: h } = info;

  // ── Locate the glyph so the menu bar isn't mostly icon padding ──────
  let minX = w, minY = h, maxX = 0, maxY = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const o = (y * w + x) * 4;
      if (data[o + 3] < 128) continue; // rounded-corner transparency
      if (luma(data[o], data[o + 1], data[o + 2]) < GLYPH_BBOX_LUMA) continue;
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    }
  }
  if (maxX < minX || maxY < minY) {
    throw new Error(`No glyph found in ${APP_ICON} — is it still a light mark on a dark plate?`);
  }

  // Square crop around the glyph, 6% breathing room.
  const side = Math.round(Math.max(maxX - minX + 1, maxY - minY + 1) * 1.06);
  const left = Math.round((minX + maxX) / 2 - side / 2);
  const top = Math.round((minY + maxY) / 2 - side / 2);

  // ── Luminance → alpha, at full resolution so the downscale antialiases it ──
  const mask = Buffer.alloc(side * side * 4, 0); // RGB stays 0 = black
  for (let y = 0; y < side; y++) {
    for (let x = 0; x < side; x++) {
      const sx = left + x;
      const sy = top + y;
      if (sx < 0 || sy < 0 || sx >= w || sy >= h) continue;
      const o = (sy * w + sx) * 4;
      const t = (luma(data[o], data[o + 1], data[o + 2]) - GLYPH_LUMA_LO) / (GLYPH_LUMA_HI - GLYPH_LUMA_LO);
      const a = Math.max(0, Math.min(1, t)) * (data[o + 3] / 255);
      mask[(y * side + x) * 4 + 3] = Math.round(a * 255);
    }
  }

  return sharp(mask, { raw: { width: side, height: side, channels: 4 } })
    .resize(TRAY_PX, TRAY_PX, { kernel: "lanczos3" })
    .png()
    .toBuffer();
}

/**
 * macOS DMG background.
 * IMPORTANT: drawn in the SAME coordinate space as tauri.conf.json `windowSize`
 * (660 × 400 points) and aligned to `appPosition` (180,200) /
 * `applicationFolderPosition` (480,200). Rendered at 144 dpi → 1320 × 800 px so
 * Finder maps it back to 660 × 400 points (crisp on Retina, no cropping).
 */
const DMG_SVG = `
<svg width="660" height="400" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="dbg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%"   stop-color="#f6f9ff"/>
      <stop offset="100%" stop-color="#e8f0fb"/>
    </linearGradient>
    <linearGradient id="darrow" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%"   stop-color="#5aa7ff"/>
      <stop offset="100%" stop-color="#2f7bf6"/>
    </linearGradient>
  </defs>

  <rect width="660" height="400" fill="url(#dbg)"/>

  <!-- Title -->
  <text x="330" y="64"
        font-family="-apple-system,'PingFang SC','Helvetica Neue',Arial,sans-serif"
        font-size="20" font-weight="600" fill="#1f2d4d" text-anchor="middle">将 Skylark 拖入「应用程序」完成安装</text>

  <!-- Curved arrow from the app icon (180,200) toward Applications (480,200) -->
  <path d="M252,208 C300,180 360,180 400,198"
        fill="none" stroke="url(#darrow)" stroke-width="8" stroke-linecap="round"/>
  <path d="M400,198 l-16,-9 l4,9 l-4,9 z" fill="#2f7bf6"/>

  <!-- Footer branding -->
  <text x="330" y="372"
        font-family="-apple-system,'PingFang SC',Arial,sans-serif"
        font-size="11" fill="#9aa7bd" text-anchor="middle">by Radium</text>
</svg>
`;

// SVG tasks: rendered at 2× then resized to target BMP size for sharpness.
// `icon` places the app icon on the banner, in 1× banner coordinates.
const TASKS = [
  {
    svg:    SIDEBAR_SVG,
    dest:   `${root}/src-tauri/installer/nsis-sidebar.bmp`,
    width:  164,
    height: 314,
    icon:   { left: 50, top: 66, size: 64 },
  },
  {
    svg:    HEADER_SVG,
    dest:   `${root}/src-tauri/installer/nsis-header.bmp`,
    width:  150,
    height: 57,
    icon:   { left: 12, top: 14, size: 30 },
  },
];

/** Banner background — also what the icon's rounded corners get flattened onto. */
const BANNER_BG = "#0f172a";

/**
 * Write a 24-bit uncompressed BMP from raw RGB pixel data.
 * NSIS expects pixels stored bottom-up (standard BMP convention).
 *
 * @param {string} dest   - output file path
 * @param {Buffer} pixels - raw RGB bytes (width × height × 3), top-down order
 * @param {number} width
 * @param {number} height
 */
function writeBmp(dest, pixels, width, height) {
  // Row size must be padded to a multiple of 4 bytes.
  const rowSize = Math.ceil((width * 3) / 4) * 4;
  const pixelDataSize = rowSize * height;
  const fileSize = 54 + pixelDataSize;

  const buf = Buffer.alloc(fileSize, 0);
  let off = 0;

  // ── File Header (14 bytes) ─────────────────────────────────────────
  buf.write("BM", off, "ascii"); off += 2;          // signature
  buf.writeUInt32LE(fileSize, off); off += 4;        // file size
  buf.writeUInt32LE(0, off); off += 4;               // reserved
  buf.writeUInt32LE(54, off); off += 4;              // pixel data offset

  // ── DIB Header / BITMAPINFOHEADER (40 bytes) ──────────────────────
  buf.writeUInt32LE(40, off); off += 4;              // header size
  buf.writeInt32LE(width, off); off += 4;            // width
  buf.writeInt32LE(height, off); off += 4;           // positive = bottom-up (required by NSIS MUI)
  buf.writeUInt16LE(1, off); off += 2;               // color planes
  buf.writeUInt16LE(24, off); off += 2;              // bits per pixel
  buf.writeUInt32LE(0, off); off += 4;               // compression (none)
  buf.writeUInt32LE(pixelDataSize, off); off += 4;   // image size
  buf.writeInt32LE(2835, off); off += 4;             // X pixels per metre (~72 dpi)
  buf.writeInt32LE(2835, off); off += 4;             // Y pixels per metre
  buf.writeUInt32LE(0, off); off += 4;               // colours in table
  buf.writeUInt32LE(0, off); off += 4;               // important colours

  // ── Pixel Data ────────────────────────────────────────────────────
  // Sharp outputs R G B; BMP uses B G R, rows stored bottom-up.
  for (let y = 0; y < height; y++) {
    const srcRow = height - 1 - y;          // flip row order: bottom-up
    for (let x = 0; x < width; x++) {
      const srcOff = (srcRow * width + x) * 3;
      const dstOff = 54 + y * rowSize + x * 3;
      buf[dstOff]     = pixels[srcOff + 2]; // B
      buf[dstOff + 1] = pixels[srcOff + 1]; // G
      buf[dstOff + 2] = pixels[srcOff];     // R
    }
  }

  mkdirSync(dirname(dest), { recursive: true });
  writeFileSync(dest, buf);
  console.log(`  ✓  ${dest.replace(root, "")}`);
}

(async () => {
  console.log("Preparing installer assets…");

  for (const { svg, dest, width, height, icon } of TASKS) {
    // Rasterize the SVG at 2×, composite the app icon, then downscale: the
    // extra resolution keeps the text and the icon's curves from stair-stepping.
    const iconPx = icon.size * SCALE;
    const iconBuf = await sharp(APP_ICON)
      .resize(iconPx, iconPx, { kernel: "lanczos3" })
      .toBuffer();

    // Two passes: sharp always composites *after* resizing within one pipeline,
    // which would drop the 2× icon onto the already-downscaled canvas.
    const banner = await sharp(Buffer.from(svg), { density: 72 * SCALE })
      .composite([{ input: iconBuf, left: icon.left * SCALE, top: icon.top * SCALE }])
      .png()
      .toBuffer();

    const { data } = await sharp(banner)
      .flatten({ background: BANNER_BG })
      .resize(width, height, { fit: "fill" })
      .raw()
      .toBuffer({ resolveWithObject: true });

    writeBmp(dest, data, width, height);
  }

  // ── macOS tray template icon (PNG, transparent, alpha-only) ────────────
  {
    const dest = `${root}/src-tauri/icons/tray-template.png`;
    writeFileSync(dest, await renderTrayTemplate());
    console.log(`  ✓  ${dest.replace(root, "")}`);
  }

  // ── macOS DMG background (PNG, 1320×800 @144dpi → 660×400 pt) ───────────
  {
    const dest = `${root}/src-tauri/installer/dmg-background.png`;
    // density:144 rasterizes the 660×400 SVG at 2× (1320×800 px); the embedded
    // 144 dpi makes Finder treat it as 660×400 points (matches windowSize).
    await sharp(Buffer.from(DMG_SVG), { density: 144 })
      .withMetadata({ density: 144 })
      .png()
      .toFile(dest);
    console.log(`  ✓  ${dest.replace(root, "")}`);
  }

  console.log("Done.\n");
})();
