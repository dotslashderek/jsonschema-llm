package com.jsonschema.llm;

import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;
import java.nio.charset.StandardCharsets;

class PanamaBinding implements Binding {
    private static final MethodHandle convertHandle;
    private static final MethodHandle rehydrateHandle;
    private static final MethodHandle freeStringHandle;

    static {
        NativeLoader.load();
        
        SymbolLookup lookup = SymbolLookup.loaderLookup();
        Linker linker = Linker.nativeLinker();
        
        // char *jsonschema_llm_convert(const char *schema_json, const char *options_json);
        convertHandle = linker.downcallHandle(
            lookup.find("jsonschema_llm_convert").orElseThrow(),
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );

        // char *jsonschema_llm_rehydrate(const char *data_json, const char *codec_json, const char *original_schema_json);
        rehydrateHandle = linker.downcallHandle(
            lookup.find("jsonschema_llm_rehydrate").orElseThrow(),
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );

        // void jsonschema_llm_free_string(char *ptr);
        freeStringHandle = linker.downcallHandle(
            lookup.find("jsonschema_llm_free_string").orElseThrow(),
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
    }

    @Override
    public String convertJson(String schemaJson, String optionsJson) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment schemaSeg = arena.allocateFrom(schemaJson);
            MemorySegment optionsSeg = arena.allocateFrom(optionsJson);

            MemorySegment resultSeg = (MemorySegment) convertHandle.invokeExact(schemaSeg, optionsSeg);
            
            if (resultSeg.equals(MemorySegment.NULL)) {
                throw new RuntimeException("Convert returned NULL");
            }
            
            resultSeg = resultSeg.reinterpret(Long.MAX_VALUE);
            String result = resultSeg.getString(0, StandardCharsets.UTF_8);
            freeStringHandle.invokeExact(resultSeg);
            
            return result;
        } catch (Throwable e) {
            throw new RuntimeException(e);
        }
    }

    @Override
    public String rehydrateJson(String dataJson, String codecJson, String originalSchemaJson) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment dataSeg = arena.allocateFrom(dataJson);
            MemorySegment codecSeg = arena.allocateFrom(codecJson);
            MemorySegment schemaSeg = arena.allocateFrom(originalSchemaJson);

            MemorySegment resultSeg = (MemorySegment) rehydrateHandle.invokeExact(dataSeg, codecSeg, schemaSeg);
            
            if (resultSeg.equals(MemorySegment.NULL)) {
                throw new RuntimeException("Rehydrate returned NULL");
            }
            
            resultSeg = resultSeg.reinterpret(Long.MAX_VALUE);
            String result = resultSeg.getString(0, StandardCharsets.UTF_8);
            freeStringHandle.invokeExact(resultSeg);
            
            return result;
        } catch (Throwable e) {
            throw new RuntimeException(e);
        }
    }
}
