import fs from 'node:fs';
import { WASI } from 'node:wasi';
const wasmPath = new URL('./target/wasm32-wasip1/release/wasitest.wasm', import.meta.url);
const args = ['wasitest', ...process.argv.slice(2)];
const wasi = new WASI({ version: 'preview1', args, returnOnExit: true, preopens: { '.': process.cwd() } });
let fuel = 2_000_000_000n;
const step = 1_500_000n;
const imports = wasi.getImportObject();
imports['traversal'] = {
  fuel_left: () => { fuel = fuel > step ? fuel - step : 0n; return fuel; },
  log: (ptr, len) => { const bytes = new Uint8Array(inst.exports.memory.buffer.slice(ptr, ptr + len)); process.stderr.write(new TextDecoder().decode(bytes) + '\n'); fuel = 2_000_000_000n; },
};
const mod = await WebAssembly.compile(fs.readFileSync(wasmPath));
const inst = await WebAssembly.instantiate(mod, imports);
wasi.start(inst);
