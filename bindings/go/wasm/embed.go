// Package wasm embeds the pre-built WASI binary for jsonschema-llm.
package wasm

import _ "embed"

//go:embed json_schema_llm_wasi.wasm
var Binary []byte
