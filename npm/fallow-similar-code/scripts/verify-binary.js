"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");

const PUBLIC_KEY = Buffer.from([
  131, 78, 111, 215, 115, 51, 230, 238, 223, 119, 147, 71, 199, 16, 172, 180, 3, 210, 216, 35, 77,
  85, 159, 94, 215, 200, 126, 85, 42, 222, 11, 209,
]);
const SPKI_HEADER = Buffer.from([
  0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
]);

const verifyBinary = (
  binaryPath,
  readFile = fs.readFileSync,
  verify = crypto.verify,
  createPublicKey = crypto.createPublicKey,
) => {
  try {
    const binary = readFile(binaryPath);
    const signature = readFile(`${binaryPath}.sig`);
    if (signature.length !== 64) {
      return { ok: false, message: "signature has an unexpected length" };
    }
    const key = createPublicKey({
      key: Buffer.concat([SPKI_HEADER, PUBLIC_KEY]),
      format: "der",
      type: "spki",
    });
    return verify(null, binary, key, signature)
      ? { ok: true }
      : { ok: false, message: "Ed25519 signature verification failed" };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { ok: false, message };
  }
};

module.exports = { verifyBinary };
