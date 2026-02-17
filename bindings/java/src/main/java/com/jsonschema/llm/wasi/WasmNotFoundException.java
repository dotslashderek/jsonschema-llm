package com.jsonschema.llm.wasi;

/**
 * Thrown when the WASM binary cannot be found through any resolution level.
 *
 * <p>
 * The message includes all locations that were searched so the caller can
 * diagnose the problem without reading source code.
 */
public class WasmNotFoundException extends RuntimeException {

    WasmNotFoundException(String sysPropValue, String envVarValue) {
        super(buildMessage(sysPropValue, envVarValue));
    }

    private static String buildMessage(String sysPropValue, String envVarValue) {
        StringBuilder sb = new StringBuilder();
        sb.append("WASM binary not found. Searched:\n");

        // Level 1: System property
        sb.append("  1. System property 'jsl.wasm.path': ");
        if (sysPropValue == null || sysPropValue.isBlank()) {
            sb.append("not set\n");
        } else {
            sb.append("set to '").append(sysPropValue).append("' — file missing\n");
        }

        // Level 2: Environment variable
        sb.append("  2. Environment variable 'JSL_WASM_PATH': ");
        if (envVarValue == null || envVarValue.isBlank()) {
            sb.append("not set\n");
        } else {
            sb.append("set to '").append(envVarValue).append("' — file missing\n");
        }

        // Level 3: Classpath
        sb.append("  3. Classpath resource '/wasm/jsonschema_llm_wasi.wasm': not found\n");

        sb.append("\nTo fix, either:\n");
        sb.append("  - Set -Djsl.wasm.path=/path/to/jsonschema_llm_wasi.wasm\n");
        sb.append("  - Set JSL_WASM_PATH=/path/to/jsonschema_llm_wasi.wasm\n");
        sb.append("  - Include the WASM binary on the classpath at /wasm/jsonschema_llm_wasi.wasm");

        return sb.toString();
    }
}
