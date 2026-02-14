package com.jsonschema.llm;

class JniBinding implements Binding {
    static {
        NativeLoader.load();
    }

    @Override
    public String convertJson(String schemaJson, String optionsJson) {
        return convert(schemaJson, optionsJson);
    }

    @Override
    public String rehydrateJson(String dataJson, String codecJson, String originalSchemaJson) {
        return rehydrate(dataJson, codecJson, originalSchemaJson);
    }

    private static native String convert(String schemaJson, String optionsJson);
    private static native String rehydrate(String dataJson, String codecJson, String originalSchemaJson);
}
