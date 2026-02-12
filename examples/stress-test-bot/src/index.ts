
import * as fs from 'fs';
import * as path from 'path';
import OpenAI from 'openai';
import Ajv from 'ajv';
import addFormats from 'ajv-formats';
import { convert, rehydrate } from 'jsonschema-llm';
import type { ResponseFormatJSONSchema } from 'openai/resources/shared';

// Initialize clients
const client = new OpenAI(); // env var OPENAI_API_KEY expected
const ajv = new Ajv({ allErrors: true, strict: false });
addFormats(ajv);

const SCHEMA_DIR = path.resolve(__dirname, '../../../tests/schemas/stress');

/** Fisher-Yates shuffle — unbiased, O(n). Fixes Finding #16. */
function fisherYatesShuffle<T>(arr: T[], seed?: number): T[] {
    const copy = [...arr];
    // Simple seeded PRNG (mulberry32)
    let s = seed ?? Math.floor(Math.random() * 2 ** 32);
    const random = () => {
        s |= 0; s = s + 0x6D2B79F5 | 0;
        let t = Math.imul(s ^ s >>> 15, 1 | s);
        t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t;
        return ((t ^ t >>> 14) >>> 0) / 4294967296;
    };
    for (let i = copy.length - 1; i > 0; i--) {
        const j = Math.floor(random() * (i + 1));
        [copy[i], copy[j]] = [copy[j], copy[i]];
    }
    return copy;
}

/** Inspect rehydrated data regardless of type. Fixes Finding #13. */
function describeData(data: unknown): string {
    if (data === null) return 'null';
    if (data === undefined) return 'undefined';
    if (Array.isArray(data)) return `array(${data.length} items)`;
    if (typeof data === 'object') return `object(${Object.keys(data).length} keys)`;
    return `${typeof data}: ${String(data).slice(0, 50)}`;
}

async function testSchema(filename: string, model: string) {
    console.log(`\n=== Testing ${filename} ===`);

    // 1. Load Schema
    const schemaPath = path.join(SCHEMA_DIR, filename);
    const originalSchema = JSON.parse(fs.readFileSync(schemaPath, 'utf-8'));

    try {
        // 2. Convert Schema (JS Binding)
        console.log(' converting...');
        const result = convert(originalSchema, {
            target: 'openai-strict',
            polymorphism: 'any-of',
            maxDepth: 50,
            recursionLimit: 3,
        });

        // Proper typing for OpenAI SDK. Fixes Finding #12.
        const jsonSchema: ResponseFormatJSONSchema.JSONSchema = {
            name: 'stress_test',
            schema: result.schema as Record<string, unknown>,
            strict: true,
        };

        // 3. Call OpenAI
        console.log(` calling ${model}...`);
        const response = await client.chat.completions.create({
            model,
            messages: [
                { role: 'system', content: 'You are a helpful assistant. Generate valid JSON.' },
                { role: 'user', content: 'Generate a complex example.' },
            ],
            response_format: {
                type: 'json_schema',
                json_schema: jsonSchema,
            },
        });

        const rawContent = response.choices[0].message.content;
        if (!rawContent) {
            throw new Error('No content from OpenAI');
        }

        const llmData = JSON.parse(rawContent);

        // 4. Rehydrate (JS Binding)
        console.log(' rehydrating...');
        const rehydrated = rehydrate(llmData, result.codec, originalSchema);

        if (rehydrated.warnings && rehydrated.warnings.length > 0) {
            console.warn(' Warnings:', rehydrated.warnings);
        }

        // 5. Validate rehydrated output against original schema. Fixes Finding #2.
        if (typeof originalSchema === 'object' && originalSchema !== null) {
            const validate = ajv.compile(originalSchema);
            const valid = validate(rehydrated.data);
            if (!valid) {
                console.error(' ❌ Rehydration validation failed:', ajv.errorsText(validate.errors));
                return false;
            }
            console.log(' ✅ Validated against original schema');
        }

        // 6. Success! Fixes Finding #13: inspect data safely.
        console.log(` ✅ Success! Rehydrated data: ${describeData(rehydrated.data)}`);
        return true;

    } catch (e: unknown) {
        const err = e as { message?: string; code?: string; path?: string; status?: number; error?: { type?: string } };
        console.error(` ❌ FAIL: ${err.message ?? String(e)}`);
        if (err.code) console.error(`    Code: ${err.code}`);
        if (err.path) console.error(`    Path: ${err.path}`);

        if (err.status === 400 && err.error?.type === 'invalid_request_error') {
            console.error('    OpenAI Schema Error Details:', JSON.stringify(err.error, null, 2));
        }
        return false;
    }
}

async function main() {
    // CLI arguments
    const args = process.argv.slice(2);
    const countIdx = args.indexOf('--count');
    const seedIdx = args.indexOf('--seed');
    const modelIdx = args.indexOf('--model');

    const count = countIdx >= 0 ? parseInt(args[countIdx + 1], 10) : 5;
    const seed = seedIdx >= 0 ? parseInt(args[seedIdx + 1], 10) : undefined;
    const model = modelIdx >= 0 ? args[modelIdx + 1] : 'gpt-4o-mini';

    if (!Number.isInteger(count) || count < 1) {
        console.error(`Error: --count must be a positive integer, got '${args[countIdx + 1]}'`);
        process.exit(1);
    }
    if (seed !== undefined && !Number.isInteger(seed)) {
        console.error(`Error: --seed must be an integer, got '${args[seedIdx + 1]}'`);
        process.exit(1);
    }

    const allFiles = fs.readdirSync(SCHEMA_DIR).filter(f => f.endsWith('.json'));
    if (allFiles.length === 0) {
        console.error(`Error: no .json files found in ${SCHEMA_DIR}`);
        process.exit(1);
    }
    const shuffled = fisherYatesShuffle(allFiles, seed);
    const testFiles = shuffled.slice(0, Math.min(count, shuffled.length));

    console.log(`Testing ${testFiles.length}/${allFiles.length} schemas (model=${model}, seed=${seed ?? 'random'})`);

    let passed = 0;
    for (const file of testFiles) {
        if (await testSchema(file, model)) {
            passed++;
        }
    }

    console.log(`\n\nSummary: ${passed}/${testFiles.length} passed.`);
    if (passed < testFiles.length) process.exit(1);
}

main().catch((err) => { console.error(err); process.exit(1); });
