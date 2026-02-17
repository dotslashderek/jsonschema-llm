package com.jsonschema.llm.wasi;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;

/**
 * Resolves the path to the WASM binary using a 3-level cascade:
 *
 * <ol>
 * <li>System property {@code jsl.wasm.path}</li>
 * <li>Environment variable {@code JSL_WASM_PATH}</li>
 * <li>Classpath resource {@code /wasm/jsonschema_llm_wasi.wasm}</li>
 * </ol>
 *
 * <p>
 * The first level that resolves to an existing, readable file wins. If none
 * succeed, a {@link WasmNotFoundException} is thrown with a descriptive
 * multi-level error message.
 *
 * <p>
 * Thread-safe: uses the initialization-on-demand holder idiom for the
 * default singleton path.
 */
class WasmResolver {

    static final String SYS_PROP_KEY = "jsl.wasm.path";
    static final String ENV_VAR_KEY = "JSL_WASM_PATH";
    static final String CLASSPATH_RESOURCE = "/wasm/jsonschema_llm_wasi.wasm";

    private WasmResolver() {
    }

    /**
     * Resolve the WASM binary path using the 3-level cascade.
     *
     * @return path to the WASM binary
     * @throws WasmNotFoundException if no level resolves successfully
     */
    static Path resolve() {
        // Level 1: System property
        String sysProp = System.getProperty(SYS_PROP_KEY);
        if (sysProp != null && !sysProp.isBlank()) {
            try {
                Path path = Path.of(sysProp);
                if (Files.isRegularFile(path) && Files.isReadable(path)) {
                    return path;
                }
            } catch (InvalidPathException ignored) {
                // Invalid path syntax — fall through to next level
            }
        }

        // Level 2: Environment variable
        String envVar = System.getenv(ENV_VAR_KEY);
        if (envVar != null && !envVar.isBlank()) {
            try {
                Path path = Path.of(envVar);
                if (Files.isRegularFile(path) && Files.isReadable(path)) {
                    return path;
                }
            } catch (InvalidPathException ignored) {
                // Invalid path syntax — fall through to next level
            }
        }

        // Level 3: Classpath resource
        Path extracted = extractFromClasspath();
        if (extracted != null) {
            return extracted;
        }

        throw new WasmNotFoundException(sysProp, envVar);
    }

    /**
     * Extract the WASM binary from the classpath to a temporary file.
     *
     * @return path to extracted temporary file, or null if not on classpath
     */
    static Path extractFromClasspath() {
        try (InputStream is = WasmResolver.class.getResourceAsStream(CLASSPATH_RESOURCE)) {
            if (is == null) {
                return null;
            }
            Path temp = Files.createTempFile("jsonschema-llm-", ".wasm");
            temp.toFile().deleteOnExit();
            Files.copy(is, temp, StandardCopyOption.REPLACE_EXISTING);
            return temp;
        } catch (IOException e) {
            throw new RuntimeException(
                    "Classpath resource " + CLASSPATH_RESOURCE + " found but extraction failed", e);
        }
    }

    /**
     * Returns the lazily-initialized default WASM path. Thread-safe via the
     * initialization-on-demand holder idiom.
     *
     * @return the resolved WASM path singleton
     * @throws WasmNotFoundException if resolution fails
     */
    static Path defaultPath() {
        return DefaultPathHolder.PATH;
    }

    /** Initialization-on-demand holder — JLS §12.4.1 guarantees thread-safety. */
    private static class DefaultPathHolder {
        static final Path PATH = resolve();
    }
}
