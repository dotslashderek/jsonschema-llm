
import * as fs from 'fs';
import * as path from 'path';
import OpenAI from 'openai';
import { convert, rehydrate } from 'jsonschema-llm';

// Initialize OpenAI client
const client = new OpenAI(); // env var OPENAI_API_KEY expected

const SCHEMA_DIR = path.resolve(__dirname, '../../../tests/schemas/stress');

async function testSchema(filename: string) {
    console.log(`\n=== Testing ${filename} ===`);
    
    // 1. Load Schema
    const schemaPath = path.join(SCHEMA_DIR, filename);
    const originalSchema = JSON.parse(fs.readFileSync(schemaPath, 'utf-8'));
    
    try {
        // 2. Convert Schema (JS Binding)
        console.log(" converting...");
        const result = convert(originalSchema, {
            target: "openai-strict",
            polymorphism: "any-of",
            maxDepth: 50,
            recursionLimit: 3
        });
        
        // 3. Call OpenAI
        console.log(" calling OpenAI...");
        const response = await client.chat.completions.create({
            model: "gpt-4o-mini",
            messages: [
                { role: "system", content: "You are a helpful assistant. Generate valid JSON." },
                { role: "user", content: "Generate a complex example." }
            ],
            response_format: {
                type: "json_schema",
                json_schema: {
                    name: "stress_test",
                    schema: result.schema as any, // Cast for OpenAI SDK compat
                    strict: true
                }
            }
        });
        
        const rawContent = response.choices[0].message.content;
        if (!rawContent) {
            throw new Error("No content from OpenAI");
        }
        
        const llmData = JSON.parse(rawContent);
        
        // 4. Rehydrate (JS Binding)
        console.log(" rehydrating...");
        const rehydrated = rehydrate(llmData, result.codec);
        
        if (rehydrated.warnings && rehydrated.warnings.length > 0) {
            console.warn(" Warnings:", rehydrated.warnings);
        }
        
        // 5. Success!
        console.log(" ✅ Success! Rehydrated data keys:", Object.keys(rehydrated.data as object));
        return true;
        
    } catch (e: any) {
        console.error(` ❌ FAIL: ${e.message}`);
        if (e.code) console.error(`    Code: ${e.code}`);
        if (e.path) console.error(`    Path: ${e.path}`);
        
        // Check for specific OpenAI errors related to schema
        if (e.status === 400 && e.error?.type === 'invalid_request_error') {
             console.error("    OpenAI Schema Error Details:", JSON.stringify(e.error, null, 2));
        }
        return false;
    }
}

async function main() {
    // Pick 5 random files
    const allFiles = fs.readdirSync(SCHEMA_DIR).filter(f => f.endsWith('.json'));
    const testFiles = allFiles
        .sort(() => 0.5 - Math.random())
        .slice(0, 5); // Test 5 random schemas
        
    let passed = 0;
    for (const file of testFiles) {
        if (await testSchema(file)) {
            passed++;
        }
    }
    
    console.log(`\n\nSummary: ${passed}/${testFiles.length} passed.`);
    if (passed < testFiles.length) process.exit(1);
}

main().catch(console.error);
