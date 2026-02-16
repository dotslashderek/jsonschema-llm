/**
 * WASI-backed wrapper for jsonschema-llm.
 *
 * Uses Node.js built-in WASI support to load the universal WASI binary
 * and exposes convert() and rehydrate() as TypeScript functions.
 *
 * Concurrency: Each Engine owns its own WASM instance. NOT thread-safe.
 */

import { readFileSync } from "node:fs";
import { WASI } from "node:wasi";
import { join } from "node:path";

const JSL_RESULT_SIZE = 12; // 3 × u32 (LE)
const STATUS_OK = 0;
const STATUS_ERROR = 1;

const DEFAULT_WASM_PATH = join(
  __dirname,
  "..",
  "..",
  "..",
  "target",
  "wasm32-wasip1",
  "release",
  "jsonschema_llm_wasi.wasm"
);

export interface ConvertOptions {
  target?: string;
  polymorphism?: string;
  max_depth?: number;
  recursion_limit?: number;
}

export interface ConvertResult {
  apiVersion: string;
  schema: Record<string, unknown>;
  codec: unknown;
}

export interface RehydrateResult {
  apiVersion: string;
  data: unknown;
  warnings?: string[];
}

export class JslError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly path: string = ""
  ) {
    super(`jsl error [${code}]${path ? ` at ${path}` : ""}: ${message}`);
    this.name = "JslError";
  }
}

export class Engine {
  private wasmBytes: Uint8Array;

  constructor(wasmPath?: string) {
    const path =
      wasmPath ?? process.env.JSL_WASM_PATH ?? DEFAULT_WASM_PATH;
    this.wasmBytes = new Uint8Array(readFileSync(path));
  }

  async convert(
    schema: unknown,
    options?: ConvertOptions
  ): Promise<ConvertResult> {
    const schemaJson = JSON.stringify(schema);
    const optsJson = JSON.stringify(options ?? {});
    const payload = await this.callJsl("jsl_convert", schemaJson, optsJson);
    return payload as ConvertResult;
  }

  async rehydrate(
    data: unknown,
    codec: unknown,
    schema: unknown
  ): Promise<RehydrateResult> {
    const dataJson = JSON.stringify(data);
    const codecJson = JSON.stringify(codec);
    const schemaJson = JSON.stringify(schema);
    const payload = await this.callJsl(
      "jsl_rehydrate",
      dataJson,
      codecJson,
      schemaJson
    );
    return payload as RehydrateResult;
  }

  private async callJsl(
    funcName: string,
    ...jsonArgs: string[]
  ): Promise<unknown> {
    // Fresh WASI instance per call
    const wasi = new WASI({ version: "preview1" });
    const wasiImports = wasi.getImportObject() as WebAssembly.Imports;
    const module = await WebAssembly.compile(this.wasmBytes as BufferSource);
    const instance = await WebAssembly.instantiate(
      module,
      wasiImports
    );
    wasi.initialize(instance);

    const exports = instance.exports as Record<string, any>;
    const memory = exports.memory as WebAssembly.Memory;
    const jslAlloc = exports.jsl_alloc as (len: number) => number;
    const jslFree = exports.jsl_free as (ptr: number, len: number) => void;
    const jslResultFree = exports.jsl_result_free as (ptr: number) => void;
    const func = exports[funcName] as (...args: number[]) => number;

    if (!func) throw new Error(`Missing export: ${funcName}`);

    // Allocate and write arguments
    const allocs: Array<{ ptr: number; len: number }> = [];
    const flatArgs: number[] = [];
    const encoder = new TextEncoder();

    for (const arg of jsonArgs) {
      const bytes = encoder.encode(arg);
      const ptr = jslAlloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
      allocs.push({ ptr, len: bytes.length });
      flatArgs.push(ptr, bytes.length);
    }

    // Call function
    const resultPtr = func(...flatArgs);
    if (resultPtr === 0) throw new Error(`${funcName} returned null`);

    // Read JslResult (12 bytes: 3 × LE u32)
    const view = new DataView(memory.buffer, resultPtr, JSL_RESULT_SIZE);
    const status = view.getUint32(0, true);
    const payloadPtr = view.getUint32(4, true);
    const payloadLen = view.getUint32(8, true);

    // Read and parse payload
    const payloadBytes = new Uint8Array(
      memory.buffer,
      payloadPtr,
      payloadLen
    );
    const payloadStr = new TextDecoder().decode(payloadBytes.slice());
    const payload = JSON.parse(payloadStr);

    // Free
    jslResultFree(resultPtr);
    for (const { ptr, len } of allocs) {
      jslFree(ptr, len);
    }

    if (status === STATUS_ERROR) {
      throw new JslError(
        payload.code ?? "unknown",
        payload.message ?? "unknown error",
        payload.path ?? ""
      );
    }

    return payload;
  }

  close(): void {
    // No persistent resources — placeholder for API symmetry
  }
}
