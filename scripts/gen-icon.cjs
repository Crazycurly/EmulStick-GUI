// Generates a 1024x1024 RGBA source PNG (no external deps) used as the input
// to `tauri icon`. Draws a rounded teal tile with a lighter "E" glyph block.
const zlib = require("zlib");
const fs = require("fs");
const path = require("path");

const SIZE = 1024;

function crc32(buf) {
  let c = ~0;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return ~c >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

function px(buf, x, y, [r, g, b, a]) {
  const o = y * (1 + SIZE * 4) + 1 + x * 4;
  buf[o] = r;
  buf[o + 1] = g;
  buf[o + 2] = b;
  buf[o + 3] = a;
}

const raw = Buffer.alloc(SIZE * (1 + SIZE * 4), 0); // filter byte 0 per row
const radius = 180;
const bg = [22, 163, 184, 255]; // teal
const fg = [230, 245, 250, 255];

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    // Rounded-rect mask.
    const cx = Math.min(x, SIZE - 1 - x);
    const cy = Math.min(y, SIZE - 1 - y);
    let inside = true;
    if (cx < radius && cy < radius) {
      const dx = radius - cx;
      const dy = radius - cy;
      inside = dx * dx + dy * dy <= radius * radius;
    }
    if (!inside) continue;
    // Draw a blocky "E".
    const inE =
      x > 320 &&
      x < 700 &&
      ((y > 300 && y < 380) || // top bar
        (y > 470 && y < 550) || // middle bar
        (y > 640 && y < 720) || // bottom bar
        (x < 400 && y > 300 && y < 720)); // spine
    px(raw, x, y, inE ? fg : bg);
  }
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // RGBA
const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", zlib.deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

const out = path.join(__dirname, "..", "src-tauri", "app-icon.png");
fs.writeFileSync(out, png);
console.log("wrote", out, png.length, "bytes");
