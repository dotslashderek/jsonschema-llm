// Package jsl provides a thin WASI-backed wrapper for jsonschema-llm.
//
// It loads the universal WASI binary via wazero (pure Go, zero CGo) and
// exposes convert() and rehydrate() as idiomatic Go functions.
//
// Architecture:
//
//	alloc guest mem → write JSON bytes → call export →
//	read JslResult (12-byte LE struct: status/ptr/len) →
//	parse JSON → free
//
// Concurrency: Each Engine owns its own wazero Runtime and Module instance.
// Engines are NOT thread-safe — callers must synchronize access or create
// per-goroutine instances.
package jsl

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"

	"github.com/dotslashderek/jsonschema-llm/bindings/go/wasm"
	"github.com/tetratelabs/wazero"
	"github.com/tetratelabs/wazero/imports/wasi_snapshot_preview1"
)

// Status codes matching the JslResult protocol.
const (
	statusOK    = 0
	statusError = 1
)

// jslResultSize is the size of the JslResult struct (3 × u32 = 12 bytes).
const jslResultSize = 12

// ConvertOptions configures schema conversion.
type ConvertOptions struct {
	Target         string `json:"target,omitempty"`
	Polymorphism   string `json:"polymorphism,omitempty"`
	MaxDepth       int    `json:"max_depth,omitempty"`
	RecursionLimit int    `json:"recursion_limit,omitempty"`
}

// ConvertResult is the result of a convert operation.
type ConvertResult struct {
	APIVersion string         `json:"apiVersion"`
	Schema     map[string]any `json:"schema"`
	Codec      any            `json:"codec"`
}

// RehydrateResult is the result of a rehydrate operation.
type RehydrateResult struct {
	APIVersion string   `json:"apiVersion"`
	Data       any      `json:"data"`
	Warnings   []string `json:"warnings,omitempty"`
}

// Error represents a structured error from the WASI binary.
type Error struct {
	Code    string `json:"code"`
	Message string `json:"message"`
	Path    string `json:"path,omitempty"`
}

func (e *Error) Error() string {
	if e.Path != "" {
		return fmt.Sprintf("jsl error [%s] at %s: %s", e.Code, e.Path, e.Message)
	}
	return fmt.Sprintf("jsl error [%s]: %s", e.Code, e.Message)
}

// Engine wraps a wazero runtime and compiled WASI module.
// Create with New(), use Convert/Rehydrate, and defer Close().
type Engine struct {
	runtime wazero.Runtime
	mod     wazero.CompiledModule
	ctx     context.Context
}

// New creates a new Engine by compiling the embedded WASI binary.
func New() (*Engine, error) {
	ctx := context.Background()
	rt := wazero.NewRuntime(ctx)

	// Instantiate WASI host functions
	if _, err := wasi_snapshot_preview1.Instantiate(ctx, rt); err != nil {
		rt.Close(ctx)
		return nil, fmt.Errorf("wasi init: %w", err)
	}

	compiled, err := rt.CompileModule(ctx, wasm.Binary)
	if err != nil {
		rt.Close(ctx)
		return nil, fmt.Errorf("compile wasm: %w", err)
	}

	return &Engine{
		runtime: rt,
		mod:     compiled,
		ctx:     ctx,
	}, nil
}

// Close releases all wazero resources.
func (e *Engine) Close() error {
	return e.runtime.Close(e.ctx)
}

// Convert transforms a JSON Schema into an LLM-compatible structured output schema.
func (e *Engine) Convert(schema any, opts *ConvertOptions) (*ConvertResult, error) {
	schemaBytes, err := json.Marshal(schema)
	if err != nil {
		return nil, fmt.Errorf("marshal schema: %w", err)
	}

	var optsBytes []byte
	if opts != nil {
		optsBytes, err = json.Marshal(opts)
		if err != nil {
			return nil, fmt.Errorf("marshal options: %w", err)
		}
	} else {
		optsBytes = []byte("{}")
	}

	payload, err := e.callJsl("jsl_convert", schemaBytes, optsBytes)
	if err != nil {
		return nil, err
	}

	var result ConvertResult
	if err := json.Unmarshal(payload, &result); err != nil {
		return nil, fmt.Errorf("unmarshal convert result: %w", err)
	}
	return &result, nil
}

// Rehydrate restores LLM output back to the original schema shape.
func (e *Engine) Rehydrate(data any, codec any, schema any) (*RehydrateResult, error) {
	dataBytes, err := json.Marshal(data)
	if err != nil {
		return nil, fmt.Errorf("marshal data: %w", err)
	}
	codecBytes, err := json.Marshal(codec)
	if err != nil {
		return nil, fmt.Errorf("marshal codec: %w", err)
	}
	schemaBytes, err := json.Marshal(schema)
	if err != nil {
		return nil, fmt.Errorf("marshal schema: %w", err)
	}

	payload, err := e.callJsl("jsl_rehydrate", dataBytes, codecBytes, schemaBytes)
	if err != nil {
		return nil, err
	}

	var result RehydrateResult
	if err := json.Unmarshal(payload, &result); err != nil {
		return nil, fmt.Errorf("unmarshal rehydrate result: %w", err)
	}
	return &result, nil
}

// callJsl executes a WASI export function following the JslResult protocol:
// alloc → write → call → read result → parse → free.
func (e *Engine) callJsl(funcName string, jsonArgs ...[]byte) ([]byte, error) {
	// Instantiate a fresh module per call (wazero modules are single-use for WASI)
	mod, err := e.runtime.InstantiateModule(e.ctx, e.mod, wazero.NewModuleConfig())
	if err != nil {
		return nil, fmt.Errorf("instantiate: %w", err)
	}
	defer mod.Close(e.ctx)

	jslAlloc := mod.ExportedFunction("jsl_alloc")
	jslFree := mod.ExportedFunction("jsl_free")
	jslResultFree := mod.ExportedFunction("jsl_result_free")
	fn := mod.ExportedFunction(funcName)

	if jslAlloc == nil || jslFree == nil || jslResultFree == nil || fn == nil {
		return nil, fmt.Errorf("missing export: %s", funcName)
	}

	// Allocate and write each argument into guest memory.
	// Note: if alloc fails mid-loop, earlier allocations are not explicitly freed.
	// This is safe because each call gets a fresh module instance (defer mod.Close
	// above), so the entire linear memory is discarded on return.
	type ptrLen struct {
		ptr uint32
		len uint32
	}
	args := make([]ptrLen, len(jsonArgs))
	for i, arg := range jsonArgs {
		results, err := jslAlloc.Call(e.ctx, uint64(len(arg)))
		if err != nil {
			return nil, fmt.Errorf("alloc: %w", err)
		}
		ptr := uint32(results[0])
		if ptr == 0 && len(arg) > 0 {
			return nil, fmt.Errorf("alloc returned null for %d bytes", len(arg))
		}
		if len(arg) > 0 {
			if !mod.Memory().Write(ptr, arg) {
				return nil, fmt.Errorf("memory write failed at ptr=%d len=%d", ptr, len(arg))
			}
		}
		args[i] = ptrLen{ptr: ptr, len: uint32(len(arg))}
	}

	// Build flat argument list: ptr, len, ptr, len, ...
	flatArgs := make([]uint64, 0, len(args)*2)
	for _, a := range args {
		flatArgs = append(flatArgs, uint64(a.ptr), uint64(a.len))
	}

	// Call the function
	results, err := fn.Call(e.ctx, flatArgs...)
	if err != nil {
		return nil, fmt.Errorf("%s trap: %w", funcName, err)
	}
	resultPtr := uint32(results[0])
	if resultPtr == 0 {
		return nil, fmt.Errorf("%s returned null result pointer", funcName)
	}

	// Read JslResult struct (12 bytes: 3 × LE u32)
	resultBytes, ok := mod.Memory().Read(resultPtr, jslResultSize)
	if !ok {
		return nil, fmt.Errorf("failed to read JslResult at ptr=%d", resultPtr)
	}
	status := binary.LittleEndian.Uint32(resultBytes[0:4])
	payloadPtr := binary.LittleEndian.Uint32(resultBytes[4:8])
	payloadLen := binary.LittleEndian.Uint32(resultBytes[8:12])

	// Read JSON payload
	payload, ok := mod.Memory().Read(payloadPtr, payloadLen)
	if !ok {
		return nil, fmt.Errorf("failed to read payload at ptr=%d len=%d", payloadPtr, payloadLen)
	}
	// Copy payload before freeing
	payloadCopy := make([]byte, len(payload))
	copy(payloadCopy, payload)

	// Free result (frees both struct and payload)
	if _, err := jslResultFree.Call(e.ctx, uint64(resultPtr)); err != nil {
		return nil, fmt.Errorf("result_free: %w", err)
	}

	// Free input buffers
	for _, a := range args {
		if _, err := jslFree.Call(e.ctx, uint64(a.ptr), uint64(a.len)); err != nil {
			return nil, fmt.Errorf("free: %w", err)
		}
	}

	// Check status
	if status == statusError {
		var jslErr Error
		if err := json.Unmarshal(payloadCopy, &jslErr); err != nil {
			return nil, fmt.Errorf("error response (unparseable): %s", string(payloadCopy))
		}
		return nil, &jslErr
	}

	return payloadCopy, nil
}
