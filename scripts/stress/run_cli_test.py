
import argparse
import subprocess
import json
import os
import sys
from openai import OpenAI
import jsonschema

def run_cli_conversion(binary_path, input_path, output_path, codec_path):
    cmd = [
        binary_path, "convert",
        input_path,
        "--output", output_path,
        "--codec", codec_path,
        "--target", "openai-strict",
        "--polymorphism", "anyof"
    ]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        return False, result.stderr
    return True, ""

def run_cli_rehydration(binary_path, input_data_path, codec_path, output_rehydrated_path):
    cmd = [
        binary_path, "rehydrate",
        input_data_path,
        "--codec", codec_path,
        "--output", output_rehydrated_path
    ]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        return False, result.stderr
    return True, ""

def call_openai(client, schema_name, schema_content):
    try:
        completion = client.chat.completions.create(
            model="gpt-4o-mini",
            messages=[
                {"role": "system", "content": "You are a helpful assistant. Generate a valid JSON object matching the provided schema. Be creative but strict."},
                {"role": "user", "content": "Generate one example."}
            ],
            response_format={
                "type": "json_schema",
                "json_schema": {
                    "name": schema_name,
                    "schema": schema_content,
                    "strict": True
                }
            }
        )
        return completion.choices[0].message.content
    except Exception as e:
        return f"OPENAI_ERROR: {str(e)}"

def validate_original(data, original_schema):
    try:
        jsonschema.validate(instance=data, schema=original_schema)
        return True, ""
    except jsonschema.ValidationError as e:
        return False, str(e)

def main():
    parser = argparse.ArgumentParser(description="Run stress tests for jsonschema-llm CLI")
    parser.add_argument("--bin", required=True, help="Path to jsonschema-llm binary")
    parser.add_argument("--schemas", required=True, help="Directory containing input schemas")
    args = parser.parse_args()

    client = OpenAI() # env var already set via source ~/.zshenv ideally, otherwise passed in
    
    schemas = [f for f in os.listdir(args.schemas) if f.endswith(".json")]
    schemas.sort()
    
    results = {"pass": [], "fail": []}
    
    print(f"Starting test run on {len(schemas)} schemas...")
    
    output_dir = "stress_results"
    os.makedirs(output_dir, exist_ok=True)
    
    for schema_file in schemas:
        base_name = os.path.splitext(schema_file)[0]
        input_path = os.path.join(args.schemas, schema_file)
        converted_path = os.path.join(output_dir, f"{base_name}.llm.json")
        codec_path = os.path.join(output_dir, f"{base_name}.codec.json")
        llm_output_path = os.path.join(output_dir, f"{base_name}.openai.json")
        rehydrated_path = os.path.join(output_dir, f"{base_name}.rehydrated.json")
        
        print(f"Testing {base_name}...", end=" ", flush=True)
        
        # 1. Convert
        success, err = run_cli_conversion(args.bin, input_path, converted_path, codec_path)
        if not success:
            print("❌ CONVERT FAIL")
            results["fail"].append({"file": schema_file, "stage": "convert", "error": err})
            continue
            
        # Load converted schema
        with open(converted_path) as f:
            llm_schema = json.load(f)
            
        # 2. OpenAI Call
        llm_response_str = call_openai(client, base_name, llm_schema)
        if llm_response_str.startswith("OPENAI_ERROR"):
            print("❌ OPENAI FAIL")
            results["fail"].append({"file": schema_file, "stage": "openai", "error": llm_response_str})
            continue
            
        # Write LLM response to file for rehydration
        with open(llm_output_path, "w") as f:
            f.write(llm_response_str)
            
        # 3. Rehydrate
        success, err = run_cli_rehydration(args.bin, llm_output_path, codec_path, rehydrated_path)
        if not success:
            print("❌ REHYDRATE FAIL")
            results["fail"].append({"file": schema_file, "stage": "rehydrate", "error": err})
            continue
            
        # 4. Validate against original
        with open(rehydrated_path) as f:
            rehydrated_data = json.load(f)
        with open(input_path) as f:
            original_schema = json.load(f)
            
        valid, err = validate_original(rehydrated_data, original_schema)
        if not valid:
            print("❌ VALIDATION FAIL")
            results["fail"].append({"file": schema_file, "stage": "validation", "error": err})
            continue
            
        print("✅ PASS")
        results["pass"].append(schema_file)

    # Summary
    print("\n=== Summary ===")
    print(f"Passed: {len(results['pass'])}")
    print(f"Failed: {len(results['fail'])}")
    
    if results["fail"]:
        print("\nFailures:")
        for fail in results["fail"]:
            print(f"- {fail['file']} ({fail['stage']}): {fail['error'][:200]}...") # truncate error
            
    with open("stress_test_report.json", "w") as f:
        json.dump(results, f, indent=2)

if __name__ == "__main__":
    main()
