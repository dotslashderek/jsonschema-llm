package com.jsonschema.llm.wasi;

/**
 * Thrown when the json-schema-llm WASM module returns an error status.
 * Contains the error code, message, and optional path to the invalid data.
 */
public class JslException extends Exception {
    private final String code;
    private final String path;

    public JslException(String code, String message, String path) {
        super(String.format("jsl error [%s]%s: %s",
                code,
                path != null && !path.isEmpty() ? " at " + path : "",
                message));
        this.code = code;
        this.path = path;
    }

    public String getCode() {
        return code;
    }

    public String getPath() {
        return path;
    }
}
