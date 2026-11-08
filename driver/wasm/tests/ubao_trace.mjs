// Trace equality in the browser build: loads the wasm package (`pkg/`, from wasm-pack for
// driver/wasm, `--target web`), runs ubao_trace/prova.as with its scenario, and compares the
// event lines the build sends to the console with ubao_trace/expected.txt, the same file
// tests/ubao_trace.rs checks natively.
//
//   node driver/wasm/tests/ubao_trace.mjs <pkg-dir>
//
// Loads the engine the way the playground's worker does (a binary .wasm, and asili_nguvu_load for
// the native module each program compiles to).

import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkgDir = resolve(process.argv[2] ?? join(here, "..", "pkg"));
const fixture = join(here, "ubao_trace");
const TAG = "\u0001ASILI-EVT ";

const lines = [];
const stdout = [];
const origLog = console.log;
console.log = (...args) => {
  const text = args.map(String).join(" ");
  if (text.startsWith(TAG)) lines.push(text.slice(TAG.length));
  else stdout.push(text);
};

// The same loader as the playground's worker.js.
globalThis.asili_nguvu_load = (bytes, count, memory, table) => {
  const base = table.grow(count);
  new WebAssembly.Instance(new WebAssembly.Module(bytes), { env: { memory, table, base } });
  return base;
};

const glue = await import(pathToFileURL(join(pkgDir, "asili_wasm.js")).href);
await glue.default({ module_or_path: readFileSync(join(pkgDir, "asili_wasm_bg.wasm")) });

const program = readFileSync(join(fixture, "prova.as"), "utf8");
const scenario = Float64Array.from(
  readFileSync(join(fixture, "scenario.txt"), "utf8").trim().split(/\s+/).map(Number),
);
glue.runBoard(program, scenario);

console.log = origLog;
const expected = readFileSync(join(fixture, "expected.txt"), "utf8").trimEnd().split("\n");
const ok =
  lines.length === expected.length && lines.every((line, i) => line === expected[i]);
if (!ok) {
  console.error("event lines differ from expected.txt:\n" + lines.join("\n"));
  process.exit(1);
}
if (stdout.join("\n") !== "saa 10") {
  console.error(`program output differs: ${JSON.stringify(stdout)}`);
  process.exit(1);
}
console.log(`ubao trace: ${lines.length} events match expected.txt; output "saa 10"`);
