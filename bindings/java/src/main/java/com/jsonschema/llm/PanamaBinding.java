package com.jsonschema.llm;

import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;
import java.nio.charset.StandardCharsets;

class PanamaBinding implements Binding {
    private static final MethodHandle convertHandle;
    private static final MethodHandle rehydrateHandle;
    private static final MethodHandle lastErrorHandle;
    private static final MethodHandle freeStringHandle;

    static {
        NativeLoader.load();

        SymbolLookup lookup = SymbolLookup.loaderLookup();
        Linker linker = Linker.nativeLinker();

        // char *jsonschema_llm_convert(const char *, const char *)
        convertHandle = linker.downcallHandle(
                lookup.find("jsonschema_llm_convert").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS));

        // char *jsonschema_llm_rehydrate(const char *, const char *, const char *)
        rehydrateHandle = linker.downcallHandle(
                lookup.find("jsonschema_llm_rehydrate").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                        ValueLayout.ADDRESS));

        // const char *jsonschema_llm_last_error()
        lastErrorHandle = linker.downcallHandle(
                lookup.find("jsonschema_llm_last_error").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.ADDRESS));

        // void jsonschema_llm_free_string(char *)
        freeStringHandle = linker.downcallHandle(
                lookup.find("jsonschema_llm_free_string").orElseThrow(),
                FunctionDescriptor.ofVoid(ValueLayout.ADDRESS));
    }

    @Override
    public String convertJson(String schemaJson, String optionsJson) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment schemaSeg = arena.allocateFrom(schemaJson);
            MemorySegment optionsSeg = arena.allocateFrom(optionsJson);

            MemorySegment resultSeg = (MemorySegment) convertHandle.invokeExact(schemaSeg, optionsSeg);

            if (resultSeg.equals(MemorySegment.NULL)) {
                throw new JsonSchemaLlmException(getLastError());
            }

            // Safe: reinterpret to max size since we only read until the null terminator.
            // The FFI contract guarantees a valid, null-terminated C string.
            resultSeg = resultSeg.reinterpret(Long.MAX_VALUE);
            try {
                return resultSeg.getString(0, StandardCharsets.UTF_8);
            } finally {
                freeStringHandle.invokeExact(resultSeg);
            }
        } catch (JsonSchemaLlmException e) {
            throw e;
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
                throw new JsonSchemaLlmException(getLastError());
            }

            // Safe: reinterpret to max size since we only read until the null terminator.
            // The FFI contract guarantees a valid, null-terminated C string.
            resultSeg = resultSeg.reinterpret(Long.MAX_VALUE);
            try {
                return resultSeg.getString(0, StandardCharsets.UTF_8);
            } finally {
                freeStringHandle.invokeExact(resultSeg);
            }
        } catch (JsonSchemaLlmException e) {
            throw e;
        } catch (Throwable e) {
            throw new RuntimeException(e);
        }
    }

    private String getLastError() {
        try {
            MemorySegment errSeg = (MemorySegment) lastErrorHandle.invokeExact();
            if (!errSeg.equals(MemorySegment.NULL)) {
                return errSeg.reinterpret(Long.MAX_VALUE).getString(0, StandardCharsets.UTF_8);
            }
            return "Unknown FFI error (no last_error available)";
        } catch (Throwable e) {
            return "Failed to retrieve last_error: " + e.getMessage();
        }
    }
}
