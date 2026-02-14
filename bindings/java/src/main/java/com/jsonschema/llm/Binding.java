package com.jsonschema.llm;

public interface Binding {
    String convertJson(String schemaJson, String optionsJson);
    String rehydrateJson(String dataJson, String codecJson, String originalSchemaJson);
}
