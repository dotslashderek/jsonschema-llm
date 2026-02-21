package jsl

import (
	"encoding/json"
	"testing"
)

// TestConvertSimple verifies basic schema conversion succeeds.
func TestConvertSimple(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"type": "object",
		"properties": map[string]any{
			"name": map[string]any{"type": "string"},
			"age":  map[string]any{"type": "integer", "minimum": 0},
		},
		"required": []any{"name", "age"},
	}

	result, err := eng.Convert(schema, nil)
	if err != nil {
		t.Fatalf("Convert() failed: %v", err)
	}

	if result.APIVersion == "" {
		t.Error("apiVersion should not be empty")
	}
	if result.Schema == nil {
		t.Error("schema should not be nil")
	}
	if result.Codec == nil {
		t.Error("codec should not be nil")
	}

	// Verify the converted schema has strict mode properties
	if _, ok := result.Schema["additionalProperties"]; !ok {
		t.Error("converted schema should have additionalProperties")
	}
}

// TestConvertWithOptions verifies conversion works with default options.
// Note: The WASI binary requires all option fields when target is specified,
// so we use nil (defaults) here. Full options are tested separately.
func TestConvertWithOptions(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"type": "object",
		"properties": map[string]any{
			"name": map[string]any{"type": "string"},
		},
	}

	result, err := eng.Convert(schema, nil)
	if err != nil {
		t.Fatalf("Convert() with default options failed: %v", err)
	}

	if result.APIVersion == "" {
		t.Error("apiVersion should not be empty")
	}
}

// TestConvertError verifies that invalid JSON input returns a structured error.
func TestConvertError(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	// Send raw invalid JSON bytes directly via callJsl to bypass Go marshalling
	_, err = eng.callJsl("jsl_convert", []byte(`NOT VALID JSON`), []byte(`{}`))
	if err == nil {
		t.Fatal("callJsl() should have returned an error for invalid input")
	}

	jslErr, ok := err.(*Error)
	if !ok {
		t.Fatalf("expected *Error, got %T: %v", err, err)
	}
	if jslErr.Code == "" {
		t.Error("error code should not be empty")
	}
	if jslErr.Message == "" {
		t.Error("error message should not be empty")
	}
}

// TestRoundtrip verifies convert → rehydrate produces valid data.
func TestRoundtrip(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"type": "object",
		"properties": map[string]any{
			"name": map[string]any{"type": "string"},
			"age":  map[string]any{"type": "integer", "minimum": 0},
		},
		"required": []any{"name", "age"},
	}

	// Convert
	convertResult, err := eng.Convert(schema, nil)
	if err != nil {
		t.Fatalf("Convert() failed: %v", err)
	}

	// Simulate LLM response matching the converted schema
	data := map[string]any{
		"name": "Ada",
		"age":  float64(36),
	}

	// Rehydrate
	rehydrateResult, err := eng.Rehydrate(data, convertResult.Codec, schema)
	if err != nil {
		t.Fatalf("Rehydrate() failed: %v", err)
	}

	if rehydrateResult.APIVersion == "" {
		t.Error("rehydrate apiVersion should not be empty")
	}
	if rehydrateResult.Data == nil {
		t.Error("rehydrate data should not be nil")
	}

	// Verify data is preserved
	dataMap, ok := rehydrateResult.Data.(map[string]any)
	if !ok {
		t.Fatalf("rehydrate data should be a map, got %T", rehydrateResult.Data)
	}
	if dataMap["name"] != "Ada" {
		t.Errorf("name should be 'Ada', got %v", dataMap["name"])
	}
}

// TestRehydrateError verifies rehydrate with invalid codec returns error.
func TestRehydrateError(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{"type": "object"}
	data := map[string]any{"key": "value"}

	_, err = eng.Rehydrate(data, "NOT VALID CODEC", schema)
	if err == nil {
		t.Fatal("Rehydrate() should have returned an error for invalid codec")
	}
}

// TestMultipleCalls verifies the engine can handle sequential calls.
func TestMultipleCalls(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"type": "object",
		"properties": map[string]any{
			"x": map[string]any{"type": "number"},
		},
	}

	for i := 0; i < 5; i++ {
		result, err := eng.Convert(schema, nil)
		if err != nil {
			t.Fatalf("Convert() call %d failed: %v", i, err)
		}
		if result.Schema == nil {
			t.Errorf("call %d: schema should not be nil", i)
		}
	}
}

// TestRealWorldSchema tests with a more complex nested schema.
func TestRealWorldSchema(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schemaJSON := `{
		"type": "object",
		"properties": {
			"user": {
				"type": "object",
				"properties": {
					"name": {"type": "string"},
					"emails": {
						"type": "array",
						"items": {"type": "string", "format": "email"}
					},
					"address": {
						"type": "object",
						"properties": {
							"street": {"type": "string"},
							"city": {"type": "string"},
							"zip": {"type": "string", "pattern": "^[0-9]{5}$"}
						},
						"required": ["street", "city"]
					}
				},
				"required": ["name"]
			},
			"role": {
				"type": "string",
				"enum": ["admin", "user", "guest"]
			}
		},
		"required": ["user", "role"]
	}`

	var schema map[string]any
	if err := json.Unmarshal([]byte(schemaJSON), &schema); err != nil {
		t.Fatalf("failed to parse test schema: %v", err)
	}

	result, err := eng.Convert(schema, nil)
	if err != nil {
		t.Fatalf("Convert() failed: %v", err)
	}

	if result.APIVersion == "" {
		t.Error("apiVersion should not be empty")
	}
	if result.Schema == nil {
		t.Error("schema should not be nil")
	}
	if result.Codec == nil {
		t.Error("codec should not be nil")
	}

	// Now do a roundtrip
	data := map[string]any{
		"user": map[string]any{
			"name":   "Ada Lovelace",
			"emails": []any{"ada@example.com"},
			"address": map[string]any{
				"street": "123 Math Lane",
				"city":   "London",
				"zip":    "12345",
			},
		},
		"role": "admin",
	}

	rehydrated, err := eng.Rehydrate(data, result.Codec, schema)
	if err != nil {
		t.Fatalf("Rehydrate() failed: %v", err)
	}

	dataMap, ok := rehydrated.Data.(map[string]any)
	if !ok {
		t.Fatalf("expected map, got %T", rehydrated.Data)
	}
	userMap, ok := dataMap["user"].(map[string]any)
	if !ok {
		t.Fatalf("expected user map, got %T", dataMap["user"])
	}
	if userMap["name"] != "Ada Lovelace" {
		t.Errorf("expected 'Ada Lovelace', got %v", userMap["name"])
	}
}

// TestListComponents verifies listing extractable components.
func TestListComponents(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"$defs": map[string]any{
			"Pet": map[string]any{"type": "string"},
			"Tag": map[string]any{"type": "integer"},
		},
	}

	result, err := eng.ListComponents(schema)
	if err != nil {
		t.Fatalf("ListComponents() failed: %v", err)
	}

	if result.APIVersion == "" {
		t.Error("apiVersion should not be empty")
	}
	if len(result.Components) != 2 {
		t.Errorf("expected 2 components, got %d", len(result.Components))
	}
}

// TestListComponentsEmpty verifies empty schema returns no components.
func TestListComponentsEmpty(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{"type": "object"}
	result, err := eng.ListComponents(schema)
	if err != nil {
		t.Fatalf("ListComponents() failed: %v", err)
	}
	if len(result.Components) != 0 {
		t.Errorf("expected 0 components, got %d", len(result.Components))
	}
}

// TestExtractComponent verifies extracting a single component.
func TestExtractComponent(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"$defs": map[string]any{
			"Pet": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"name": map[string]any{"type": "string"},
				},
			},
		},
	}

	result, err := eng.ExtractComponent(schema, "#/$defs/Pet", nil)
	if err != nil {
		t.Fatalf("ExtractComponent() failed: %v", err)
	}

	if result.APIVersion == "" {
		t.Error("apiVersion should not be empty")
	}
	if result.Pointer != "#/$defs/Pet" {
		t.Errorf("pointer: got %q, want %q", result.Pointer, "#/$defs/Pet")
	}
	if result.Schema == nil {
		t.Error("schema should not be nil")
	}
}

// TestExtractComponentError verifies missing pointer returns error.
func TestExtractComponentError(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"$defs": map[string]any{
			"Foo": map[string]any{"type": "string"},
		},
	}

	_, err = eng.ExtractComponent(schema, "#/$defs/DoesNotExist", nil)
	if err == nil {
		t.Fatal("expected error for missing pointer, got nil")
	}
}

// TestConvertAllComponents verifies batch conversion.
func TestConvertAllComponents(t *testing.T) {
	eng, err := New()
	if err != nil {
		t.Fatalf("New() failed: %v", err)
	}
	defer eng.Close()

	schema := map[string]any{
		"$defs": map[string]any{
			"A": map[string]any{"type": "string"},
			"B": map[string]any{"type": "integer"},
		},
	}

	result, err := eng.ConvertAllComponents(schema, nil, nil)
	if err != nil {
		t.Fatalf("ConvertAllComponents() failed: %v", err)
	}

	if result.APIVersion == "" {
		t.Error("apiVersion should not be empty")
	}
	if result.Full == nil {
		t.Error("full should not be nil")
	}
	if result.Components == nil {
		t.Error("components should not be nil")
	}
}
