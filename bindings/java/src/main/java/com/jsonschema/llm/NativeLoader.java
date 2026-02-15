package com.jsonschema.llm;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

public class NativeLoader {
    private static volatile boolean loaded = false;

    public static void load() {
        if (loaded) {
            return;
        }
        synchronized (NativeLoader.class) {
            if (loaded) {
                return;
            }
            try {
                loadLibrary();
                loaded = true;
            } catch (Exception e) {
                throw new RuntimeException("Failed to load native library", e);
            }
        }
    }

    private static void loadLibrary() throws IOException {
        String os = System.getProperty("os.name").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch").toLowerCase(Locale.ROOT);

        String mappedOs;
        String mappedArch;
        String libName;

        if (os.contains("mac")) {
            mappedOs = "darwin";
            libName = "libjsonschema_llm_java.dylib";
        } else if (os.contains("win")) {
            mappedOs = "windows";
            libName = "jsonschema_llm_java.dll";
        } else {
            mappedOs = "linux";
            libName = "libjsonschema_llm_java.so";
        }

        if (arch.equals("amd64") || arch.equals("x86_64")) {
            mappedArch = "x86_64";
        } else if (arch.equals("aarch64") || arch.equals("arm64")) {
            mappedArch = "aarch64";
        } else {
            throw new UnsupportedOperationException("Unsupported architecture: " + arch);
        }

        String resourcePath = "/native/" + mappedOs + "-" + mappedArch + "/" + libName;
        try (InputStream is = NativeLoader.class.getResourceAsStream(resourcePath)) {
            if (is == null) {
                throw new IOException("Native library not found at: " + resourcePath);
            }

            // Create a temporary directory for this JVM instance.
            // Only called once due to the volatile `loaded` guard above.
            Path tempDir = Files.createTempDirectory("jsonschema-llm-java");
            Path tempFile = tempDir.resolve(libName);

            Files.copy(is, tempFile, StandardCopyOption.REPLACE_EXISTING);
            tempFile.toFile().deleteOnExit();
            tempDir.toFile().deleteOnExit();

            System.load(tempFile.toAbsolutePath().toString());
        }
    }
}
