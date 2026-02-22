// Go stress test bot for json-schema-llm WASI wrapper.
//
// Pipeline: convert → OpenAI structured output → rehydrate → validate
// Mirrors the TS/Python/Java reference clients.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	jsl "github.com/dotslashderek/json-schema-llm/bindings/go"
	"github.com/openai/openai-go"
	"github.com/openai/openai-go/option"
	"github.com/santhosh-tekuri/jsonschema/v6"
)

func main() {
	count := flag.Int("count", 0, "Number of schemas to test (0 = all)")
	seed := flag.Int("seed", 0, "Random seed for schema selection")
	model := flag.String("model", "gpt-4o-mini", "OpenAI model to use")
	schemasDir := flag.String("schemas-dir", "", "Path to schemas directory")
	flag.Parse()

	if *schemasDir == "" {
		// Default: relative to this binary
		*schemasDir = filepath.Join("..", "..", "tests", "schemas")
	}

	// Load schemas
	schemas, err := loadSchemas(*schemasDir)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to load schemas: %v\n", err)
		os.Exit(1)
	}

	// Shuffle with deterministic PRNG
	if *seed != 0 {
		shuffle(schemas, uint32(*seed))
	}

	if *count > 0 && *count < len(schemas) {
		schemas = schemas[:*count]
	}

	fmt.Printf("🤖 Go Stress Test Bot\n")
	fmt.Printf("   Model: %s\n", *model)
	fmt.Printf("   Schemas: %d\n", len(schemas))
	fmt.Printf("   Seed: %d\n\n", *seed)

	// Initialize engine
	engine, err := jsl.New()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to initialize WASI engine: %v\n", err)
		os.Exit(1)
	}
	defer engine.Close()

	// Initialize OpenAI client
	client := openai.NewClient(option.WithAPIKey(os.Getenv("OPENAI_API_KEY")))

	passed := 0
	failed := 0
	var totalElapsed time.Duration

	for i, s := range schemas {
		fmt.Printf("[%d/%d] %s ... ", i+1, len(schemas), s.name)
		ok, elapsed, testErr := testSchema(engine, client, s, *model)
		totalElapsed += elapsed
		if ok {
			passed++
			fmt.Printf("✅ (%.2fs)\n", elapsed.Seconds())
		} else {
			failed++
			fmt.Printf("❌ %v\n", testErr)
		}
	}

	fmt.Printf("\n📊 Results: %d passed, %d failed, %.2fs total\n",
		passed, failed, totalElapsed.Seconds())

	if failed > 0 {
		os.Exit(1)
	}
}

type schemaEntry struct {
	name   string
	schema map[string]any
}

func loadSchemas(dir string) ([]schemaEntry, error) {
	var entries []schemaEntry

	// Load from all subdirectories
	subdirs := []string{"", "real-world"}
	for _, sub := range subdirs {
		searchDir := filepath.Join(dir, sub)
		files, err := os.ReadDir(searchDir)
		if err != nil {
			if os.IsNotExist(err) {
				continue
			}
			return nil, err
		}
		for _, f := range files {
			if !strings.HasSuffix(f.Name(), ".json") {
				continue
			}
			data, err := os.ReadFile(filepath.Join(searchDir, f.Name()))
			if err != nil {
				return nil, err
			}
			var schema map[string]any
			if err := json.Unmarshal(data, &schema); err != nil {
				continue // skip non-object schemas
			}
			name := f.Name()
			if sub != "" {
				name = sub + "/" + name
			}
			entries = append(entries, schemaEntry{name: name, schema: schema})
		}
	}

	sort.Slice(entries, func(i, j int) bool {
		return entries[i].name < entries[j].name
	})
	return entries, nil
}

func testSchema(
	engine *jsl.Engine,
	client *openai.Client,
	s schemaEntry,
	model string,
) (bool, time.Duration, error) {
	start := time.Now()

	// 1. Convert
	convertResult, err := engine.Convert(s.schema, nil)
	if err != nil {
		return false, time.Since(start), fmt.Errorf("convert: %w", err)
	}

	// 2. Call OpenAI
	convertedSchemaBytes, err := json.Marshal(convertResult.Schema)
	if err != nil {
		return false, time.Since(start), fmt.Errorf("marshal converted schema: %w", err)
	}
	var schemaParam map[string]any
	if err := json.Unmarshal(convertedSchemaBytes, &schemaParam); err != nil {
		return false, time.Since(start), fmt.Errorf("unmarshal schema param: %w", err)
	}

	resp, err := client.Chat.Completions.New(context.Background(),
		openai.ChatCompletionNewParams{
			Model: openai.F(model),
			Messages: openai.F([]openai.ChatCompletionMessageParamUnion{
				openai.SystemMessage("Generate realistic sample data matching the provided JSON schema. Be creative but realistic."),
				openai.UserMessage(fmt.Sprintf("Generate data for this schema: %s", string(convertedSchemaBytes))),
			}),
			ResponseFormat: openai.F[openai.ChatCompletionNewParamsResponseFormatUnion](
				openai.ResponseFormatJSONSchemaParam{
					Type: openai.F(openai.ResponseFormatJSONSchemaTypeJSONSchema),
					JSONSchema: openai.F(openai.ResponseFormatJSONSchemaJSONSchemaParam{
						Name:   openai.F("response"),
						Schema: openai.F(any(schemaParam)),
						Strict: openai.F(true),
					}),
				},
			),
		},
	)
	if err != nil {
		return false, time.Since(start), fmt.Errorf("openai: %w", err)
	}

	if len(resp.Choices) == 0 {
		return false, time.Since(start), fmt.Errorf("openai: empty choices")
	}

	content := resp.Choices[0].Message.Content
	var llmData any
	if err := json.Unmarshal([]byte(content), &llmData); err != nil {
		return false, time.Since(start), fmt.Errorf("parse llm response: %w", err)
	}

	// 3. Rehydrate
	rehydrateResult, err := engine.Rehydrate(llmData, convertResult.Codec, s.schema)
	if err != nil {
		return false, time.Since(start), fmt.Errorf("rehydrate: %w", err)
	}

	// 4. Validate
	rehydratedBytes, err := json.Marshal(rehydrateResult.Data)
	if err != nil {
		return false, time.Since(start), fmt.Errorf("marshal rehydrated: %w", err)
	}
	schemaBytes, err := json.Marshal(s.schema)
	if err != nil {
		return false, time.Since(start), fmt.Errorf("marshal schema: %w", err)
	}

	compiler := jsonschema.NewCompiler()
	if err := compiler.AddResource("schema.json", strings.NewReader(string(schemaBytes))); err != nil {
		return false, time.Since(start), fmt.Errorf("add schema: %w", err)
	}
	sch, err := compiler.Compile("schema.json")
	if err != nil {
		return false, time.Since(start), fmt.Errorf("compile schema: %w", err)
	}

	var rehydratedAny any
	if err := json.Unmarshal(rehydratedBytes, &rehydratedAny); err != nil {
		return false, time.Since(start), fmt.Errorf("unmarshal rehydrated: %w", err)
	}
	if err := sch.Validate(rehydratedAny); err != nil {
		return false, time.Since(start), fmt.Errorf("validate: %w", err)
	}

	return true, time.Since(start), nil
}

// Mulberry32 PRNG + Fisher-Yates shuffle for deterministic ordering
func shuffle(entries []schemaEntry, seed uint32) {
	for i := len(entries) - 1; i > 0; i-- {
		seed += 0x6D2B79F5
		t := seed
		t = (t ^ (t >> 15)) * (t | 1)
		t ^= t + (t^(t>>7))*(t|61)
		t = t ^ (t >> 14)
		j := int(t % uint32(i+1))
		entries[i], entries[j] = entries[j], entries[i]
	}
}
