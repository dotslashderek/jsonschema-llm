package jsl

import (
	"encoding/json"
	"os"
	"testing"
)

// fixtureFile is the top-level structure of fixtures.json.
type fixtureFile struct {
	Suites map[string]suite `json:"suites"`
}

type suite struct {
	Description string    `json:"description"`
	Fixtures    []fixture `json:"fixtures"`
}

type fixture struct {
	ID          string         `json:"id"`
	Description string         `json:"description"`
	Input       fixtureInput   `json:"input"`
	Expected    map[string]any `json:"expected"`
}

type fixtureInput struct {
	Schema         any            `json:"schema,omitempty"`
	SchemaRaw      string         `json:"schema_raw,omitempty"`
	Options        map[string]any `json:"options,omitempty"`
	Data           any            `json:"data,omitempty"`
	CodecRaw       string         `json:"codec_raw,omitempty"`
	Pointer        string         `json:"pointer,omitempty"`
	ConvertOptions map[string]any `json:"convert_options,omitempty"`
	ExtractOptions map[string]any `json:"extract_options,omitempty"`
}

func loadFixtures(t *testing.T) fixtureFile {
	t.Helper()
	data, err := os.ReadFile("../../tests/conformance/fixtures.json")
	if err != nil {
		t.Fatalf("failed to load fixtures.json: %v", err)
	}
	var f fixtureFile
	if err := json.Unmarshal(data, &f); err != nil {
		t.Fatalf("failed to parse fixtures.json: %v", err)
	}
	return f
}

// fixtureOptionsToConvertOptions maps fixture options (kebab-case JSON) to the
// Go ConvertOptions struct, which is how real users interact with the library.
func fixtureOptionsToConvertOptions(t *testing.T, opts map[string]any) *ConvertOptions {
	t.Helper()
	if len(opts) == 0 {
		return nil
	}
	b, err := json.Marshal(opts)
	if err != nil {
		t.Fatalf("fixtureOptionsToConvertOptions: marshal failed: %v", err)
	}
	co := &ConvertOptions{}
	if err := json.Unmarshal(b, co); err != nil {
		t.Fatalf("fixtureOptionsToConvertOptions: unmarshal failed: %v", err)
	}
	return co
}

func TestConformance_Convert(t *testing.T) {
	fixtures := loadFixtures(t)
	convertSuite := fixtures.Suites["convert"]

	for _, fx := range convertSuite.Fixtures {
		t.Run(fx.ID, func(t *testing.T) {
			eng, err := New()
			if err != nil {
				t.Fatalf("New() failed: %v", err)
			}
			defer eng.Close()

			expected := fx.Expected

			// Error case: schema_raw present → raw FFI call
			if fx.Input.SchemaRaw != "" {
				isError, _ := expected["is_error"].(bool)
				if !isError {
					t.Fatal("schema_raw fixture must have is_error=true")
				}

				optsJSON := "{}"
				if fx.Input.Options != nil {
					b, err := json.Marshal(fx.Input.Options)
					if err != nil {
						t.Fatalf("failed to marshal options: %v", err)
					}
					optsJSON = string(b)
				}
				_, err := eng.callJsl("jsl_convert", []byte(fx.Input.SchemaRaw), []byte(optsJSON))
				if err == nil {
					t.Fatal("expected error for schema_raw fixture, got nil")
				}

				jslErr, ok := err.(*Error)
				if !ok {
					t.Fatalf("expected *Error, got %T: %v", err, err)
				}

				assertErrorExpected(t, jslErr, expected)
				return
			}

			// Normal convert: use the high-level Convert() API with ConvertOptions
			opts := fixtureOptionsToConvertOptions(t, fx.Input.Options)
			result, err := eng.Convert(fx.Input.Schema, opts)
			if err != nil {
				t.Fatalf("Convert() failed: %v", err)
			}

			assertConvertExpected(t, result, expected)
		})
	}
}

func TestConformance_Roundtrip(t *testing.T) {
	fixtures := loadFixtures(t)
	roundtripSuite := fixtures.Suites["roundtrip"]

	for _, fx := range roundtripSuite.Fixtures {
		t.Run(fx.ID, func(t *testing.T) {
			eng, err := New()
			if err != nil {
				t.Fatalf("New() failed: %v", err)
			}
			defer eng.Close()

			expected := fx.Expected

			// Convert via the high-level API
			opts := fixtureOptionsToConvertOptions(t, fx.Input.Options)
			convertResult, err := eng.Convert(fx.Input.Schema, opts)
			if err != nil {
				t.Fatalf("Convert() failed: %v", err)
			}

			// Rehydrate via the high-level API
			rehydrateResult, err := eng.Rehydrate(fx.Input.Data, convertResult.Codec, fx.Input.Schema)
			if err != nil {
				t.Fatalf("Rehydrate() failed: %v", err)
			}

			// Assert has_keys
			if keys, ok := expected["has_keys"].([]any); ok {
				resultMap := map[string]any{
					"apiVersion": rehydrateResult.APIVersion,
					"data":       rehydrateResult.Data,
					"warnings":   rehydrateResult.Warnings,
				}
				for _, k := range keys {
					key := k.(string)
					val, exists := resultMap[key]
					if !exists || val == nil {
						t.Errorf("result missing key %q (or value is nil)", key)
					}
				}
			}

			// Assert apiVersion
			if v, ok := expected["apiVersion"].(string); ok {
				if rehydrateResult.APIVersion != v {
					t.Errorf("apiVersion: got %q, want %q", rehydrateResult.APIVersion, v)
				}
			}

			// Assert data deep-equals
			if expectedData, ok := expected["data"]; ok {
				actualJSON, err := json.Marshal(rehydrateResult.Data)
				if err != nil {
					t.Fatalf("data: failed to marshal actual: %v", err)
				}
				expectedJSON, err := json.Marshal(expectedData)
				if err != nil {
					t.Fatalf("data: failed to marshal expected: %v", err)
				}
				if string(actualJSON) != string(expectedJSON) {
					t.Errorf("data mismatch:\n  got:  %s\n  want: %s", actualJSON, expectedJSON)
				}
			}

			// Assert data_user_name
			if name, ok := expected["data_user_name"].(string); ok {
				dataMap, ok := rehydrateResult.Data.(map[string]any)
				if !ok {
					t.Fatalf("data_user_name: expected data to be map[string]any, got %T", rehydrateResult.Data)
				}
				userMap, ok := dataMap["user"].(map[string]any)
				if !ok {
					t.Fatalf("data_user_name: expected data[\"user\"] to be map[string]any, got %T", dataMap["user"])
				}
				if userMap["name"] != name {
					t.Errorf("data.user.name: got %v, want %q", userMap["name"], name)
				}
			}

			// Assert data_value
			if val, ok := expected["data_value"]; ok {
				dataMap, ok := rehydrateResult.Data.(map[string]any)
				if !ok {
					t.Fatalf("data_value: expected data to be map[string]any, got %T", rehydrateResult.Data)
				}
				actualJSON, err := json.Marshal(dataMap["value"])
				if err != nil {
					t.Fatalf("data_value: failed to marshal actual: %v", err)
				}
				expectedJSON, err := json.Marshal(val)
				if err != nil {
					t.Fatalf("data_value: failed to marshal expected: %v", err)
				}
				if string(actualJSON) != string(expectedJSON) {
					t.Errorf("data.value: got %s, want %s", actualJSON, expectedJSON)
				}
			}

			// Assert warnings_is_array
			if _, ok := expected["warnings_is_array"]; ok {
				if rehydrateResult.Warnings == nil {
					t.Error("warnings is nil, expected non-nil array")
				}
			}
		})
	}
}

func TestConformance_RehydrateError(t *testing.T) {
	fixtures := loadFixtures(t)
	errorSuite := fixtures.Suites["rehydrate_error"]

	for _, fx := range errorSuite.Fixtures {
		t.Run(fx.ID, func(t *testing.T) {
			eng, err := New()
			if err != nil {
				t.Fatalf("New() failed: %v", err)
			}
			defer eng.Close()

			expected := fx.Expected

			dataBytes, err := json.Marshal(fx.Input.Data)
			if err != nil {
				t.Fatalf("failed to marshal input data: %v", err)
			}
			schemaBytes, err := json.Marshal(fx.Input.Schema)
			if err != nil {
				t.Fatalf("failed to marshal input schema: %v", err)
			}

			var codecArg []byte
			if fx.Input.CodecRaw != "" {
				codecArg = []byte(fx.Input.CodecRaw)
			} else {
				t.Fatal("rehydrate_error fixture must have codec_raw")
			}

			_, err = eng.callJsl("jsl_rehydrate", dataBytes, codecArg, schemaBytes)
			if err == nil {
				t.Fatal("expected error for rehydrate_error fixture, got nil")
			}

			isError, _ := expected["is_error"].(bool)
			if !isError {
				t.Fatal("rehydrate_error fixture must have is_error=true")
			}

			jslErr, ok := err.(*Error)
			if !ok {
				t.Fatalf("expected *Error, got %T: %v", err, err)
			}

			assertErrorExpected(t, jslErr, expected)
		})
	}
}

// assertConvertExpected validates convert result against expected assertions.
func assertConvertExpected(t *testing.T, result *ConvertResult, expected map[string]any) {
	t.Helper()

	// has_keys
	if keys, ok := expected["has_keys"].([]any); ok {
		resultMap := map[string]any{
			"apiVersion": result.APIVersion,
			"schema":     result.Schema,
			"codec":      result.Codec,
		}
		for _, k := range keys {
			key := k.(string)
			if val, exists := resultMap[key]; !exists || val == nil {
				t.Errorf("result missing key %q", key)
			}
		}
	}

	// apiVersion
	if v, ok := expected["apiVersion"].(string); ok {
		if result.APIVersion != v {
			t.Errorf("apiVersion: got %q, want %q", result.APIVersion, v)
		}
	}

	// schema_has_properties
	if _, ok := expected["schema_has_properties"]; ok {
		if _, exists := result.Schema["properties"]; !exists {
			t.Error("schema missing 'properties'")
		}
	}

	// codec_has_schema_uri
	if _, ok := expected["codec_has_schema_uri"]; ok {
		if result.Codec == nil {
			t.Error("codec is nil")
		}
	}
}

// assertErrorExpected validates error fields against expected assertions.
func assertErrorExpected(t *testing.T, jslErr *Error, expected map[string]any) {
	t.Helper()

	if keys, ok := expected["error_has_keys"].([]any); ok {
		for _, k := range keys {
			key := k.(string)
			switch key {
			case "code":
				if jslErr.Code == "" {
					t.Error("error missing 'code'")
				}
			case "message":
				if jslErr.Message == "" {
					t.Error("error missing 'message'")
				}
			}
		}
	}

	if code, ok := expected["error_code"].(string); ok {
		if jslErr.Code != code {
			t.Errorf("error_code: got %q, want %q", jslErr.Code, code)
		}
	}
}

func TestConformance_ListComponents(t *testing.T) {
	fixtures := loadFixtures(t)
	listSuite := fixtures.Suites["list_components"]

	for _, fx := range listSuite.Fixtures {
		t.Run(fx.ID, func(t *testing.T) {
			eng, err := New()
			if err != nil {
				t.Fatalf("New() failed: %v", err)
			}
			defer eng.Close()

			expected := fx.Expected

			// Error case: schema_raw
			if fx.Input.SchemaRaw != "" {
				_, err := eng.callJsl("jsl_list_components", []byte(fx.Input.SchemaRaw))
				if err == nil {
					t.Fatal("expected error for schema_raw fixture, got nil")
				}
				jslErr, ok := err.(*Error)
				if !ok {
					t.Fatalf("expected *Error, got %T: %v", err, err)
				}
				assertErrorExpected(t, jslErr, expected)
				return
			}

			result, err := eng.ListComponents(fx.Input.Schema)
			if err != nil {
				t.Fatalf("ListComponents() failed: %v", err)
			}

			// apiVersion
			if v, ok := expected["apiVersion"].(string); ok {
				if result.APIVersion != v {
					t.Errorf("apiVersion: got %q, want %q", result.APIVersion, v)
				}
			}

			// components exact match
			if comps, ok := expected["components"]; ok {
				expectedJSON, _ := json.Marshal(comps)
				actualJSON, _ := json.Marshal(result.Components)
				if string(actualJSON) != string(expectedJSON) {
					t.Errorf("components mismatch:\n  got:  %s\n  want: %s", actualJSON, expectedJSON)
				}
			}
		})
	}
}

func TestConformance_ExtractComponent(t *testing.T) {
	fixtures := loadFixtures(t)
	extractSuite := fixtures.Suites["extract_component"]

	for _, fx := range extractSuite.Fixtures {
		t.Run(fx.ID, func(t *testing.T) {
			eng, err := New()
			if err != nil {
				t.Fatalf("New() failed: %v", err)
			}
			defer eng.Close()

			expected := fx.Expected

			// Error case
			if isErr, _ := expected["is_error"].(bool); isErr {
				if fx.Input.SchemaRaw != "" {
					_, err := eng.callJsl("jsl_extract_component", []byte(fx.Input.SchemaRaw), []byte(fx.Input.Pointer), []byte("{}"))
					if err == nil {
						t.Fatal("expected error, got nil")
					}
					jslErr, ok := err.(*Error)
					if !ok {
						t.Fatalf("expected *Error, got %T: %v", err, err)
					}
					assertErrorExpected(t, jslErr, expected)
				} else {
					_, err := eng.ExtractComponent(fx.Input.Schema, fx.Input.Pointer, nil)
					if err == nil {
						t.Fatal("expected error, got nil")
					}
					jslErr, ok := err.(*Error)
					if !ok {
						t.Fatalf("expected *Error, got %T: %v", err, err)
					}
					assertErrorExpected(t, jslErr, expected)
				}
				return
			}

			result, err := eng.ExtractComponent(fx.Input.Schema, fx.Input.Pointer, nil)
			if err != nil {
				t.Fatalf("ExtractComponent() failed: %v", err)
			}

			// apiVersion
			if v, ok := expected["apiVersion"].(string); ok {
				if result.APIVersion != v {
					t.Errorf("apiVersion: got %q, want %q", result.APIVersion, v)
				}
			}
			// pointer
			if v, ok := expected["pointer"].(string); ok {
				if result.Pointer != v {
					t.Errorf("pointer: got %q, want %q", result.Pointer, v)
				}
			}
			// schema_is_object
			if _, ok := expected["schema_is_object"]; ok {
				if result.Schema == nil {
					t.Error("schema should not be nil")
				}
			}
			// dependency_count
			if v, ok := expected["dependency_count"]; ok {
				wantCount := int(v.(float64))
				if result.DependencyCount != wantCount {
					t.Errorf("dependencyCount: got %d, want %d", result.DependencyCount, wantCount)
				}
			}
			// dependency_count_gte
			if v, ok := expected["dependency_count_gte"]; ok {
				min := int(v.(float64))
				if result.DependencyCount < min {
					t.Errorf("dependencyCount: got %d, want >= %d", result.DependencyCount, min)
				}
			}
		})
	}
}

func TestConformance_ConvertAllComponents(t *testing.T) {
	fixtures := loadFixtures(t)
	convertAllSuite := fixtures.Suites["convert_all_components"]

	for _, fx := range convertAllSuite.Fixtures {
		t.Run(fx.ID, func(t *testing.T) {
			eng, err := New()
			if err != nil {
				t.Fatalf("New() failed: %v", err)
			}
			defer eng.Close()

			expected := fx.Expected

			// Error case: schema_raw
			if fx.Input.SchemaRaw != "" {
				convBytes, _ := json.Marshal(fx.Input.ConvertOptions)
				extBytes, _ := json.Marshal(fx.Input.ExtractOptions)
				if convBytes == nil {
					convBytes = []byte("{}")
				}
				if extBytes == nil {
					extBytes = []byte("{}")
				}
				_, err := eng.callJsl("jsl_convert_all_components", []byte(fx.Input.SchemaRaw), convBytes, extBytes)
				if err == nil {
					t.Fatal("expected error for schema_raw fixture, got nil")
				}
				jslErr, ok := err.(*Error)
				if !ok {
					t.Fatalf("expected *Error, got %T: %v", err, err)
				}
				assertErrorExpected(t, jslErr, expected)
				return
			}

			result, err := eng.ConvertAllComponents(fx.Input.Schema, nil, nil)
			if err != nil {
				t.Fatalf("ConvertAllComponents() failed: %v", err)
			}

			// apiVersion
			if v, ok := expected["apiVersion"].(string); ok {
				if result.APIVersion != v {
					t.Errorf("apiVersion: got %q, want %q", result.APIVersion, v)
				}
			}
			// full_is_object
			if _, ok := expected["full_is_object"]; ok {
				if result.Full == nil {
					t.Error("full should not be nil")
				}
			}
			// components_count
			if v, ok := expected["components_count"]; ok {
				wantCount := int(v.(float64))
				var comps []any
				if err := json.Unmarshal(result.Components, &comps); err != nil {
					t.Fatalf("failed to parse components: %v", err)
				}
				if len(comps) != wantCount {
					t.Errorf("components count: got %d, want %d", len(comps), wantCount)
				}
			}
		})
	}
}
