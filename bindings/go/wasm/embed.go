// Package wasm embeds the pre-built WASI binary for jsonschema-llm.
package wasm

import _ "embed"

//go:embed jsonschema_llm_wasi.wasm
var Binary []byte
