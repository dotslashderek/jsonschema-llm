package com.jsonschema.llm;

public class JsonSchemaLlmException extends RuntimeException {
    private final String code;
    private final String path;

    public JsonSchemaLlmException(String message, String code, String path) {
        super(message);
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
