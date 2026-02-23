# frozen_string_literal: true

# LLM Roundtrip Engine — orchestrates convert → format → call → rehydrate → validate.
#
# The wasmtime gem is loaded lazily — only needed when the engine is instantiated.
# This allows non-WASI tests (types, formatter, etc.) to run on any Ruby version.

require "json"

# Optional JSON Schema validation


module JsonSchemaLlmEngine
  # Orchestrates the full LLM roundtrip.
  #
  # Lifecycle: convert schema → format request → call LLM → extract content →
  # rehydrate output → validate against original schema.
  #
  # The wasmtime Engine and compiled Module are cached at init time.
  # A fresh Store + Instance is created per call (WASI modules are single-use).
  #
  # @example
  #   engine = LlmRoundtripEngine.new(
  #     formatter: Formatters::ChatCompletions.new,
  #     config: ProviderConfig.new(url: "https://api.openai.com/v1/chat/completions", model: "gpt-4o"),
  #     transport: my_http_transport,
  #   )
  #   result = engine.generate(schema_json, "Generate a user profile")
  class LlmRoundtripEngine
    JSON_SCHEMER_AVAILABLE = begin
      require "json_schemer"
      true
    rescue LoadError
      false
    end

    EXPECTED_ABI_VERSION = 1
    JSL_RESULT_SIZE = 12 # 3 × u32 (LE)
    STATUS_OK = 0
    STATUS_ERROR = 1

    # @param formatter [#format, #extract_content] provider-specific request formatter
    # @param config [ProviderConfig] provider endpoint/model configuration
    # @param transport [#execute] consumer-provided HTTP transport
    # @param wasm_path [String, nil] explicit path to the WASI binary (auto-discovers if nil)
    def initialize(formatter:, config:, transport:, wasm_path: nil)
      validate_formatter!(formatter)
      validate_transport!(transport)

      @formatter = formatter
      @config = config
      @transport = transport

      # Lazy-load wasmtime gem — only needed at engine instantiation time
      require "wasmtime"

      wasm_bytes = resolve_wasm_bytes(wasm_path)
      @wasm_engine = Wasmtime::Engine.new
      @module = Wasmtime::Module.new(@wasm_engine, wasm_bytes)
    end

    # Creates an engine, yields it to the block, and ensures it is closed afterward.
    #
    # @param kwargs arguments passed to {#initialize}
    # @yieldparam engine [LlmRoundtripEngine]
    def self.open(**kwargs)
      engine = new(**kwargs)
      begin
        yield engine
      ensure
        engine.close
      end
    end

    # Execute a full roundtrip: convert → format → call LLM → rehydrate → validate.
    #
    # @param schema_json [String] the original JSON Schema as a string
    # @param prompt [String] the natural language prompt for the LLM
    # @return [RoundtripResult]
    # @raise [SchemaConversionError] if schema conversion fails
    # @raise [LlmTransportError] if the transport fails
    def generate(schema_json, prompt)
      # Step 1: Convert schema to LLM-compatible form
      convert_result = call_wasi("jsl_convert", schema_json, "{}")
      llm_schema = convert_result["schema"] || {}
      codec = convert_result["codec"] || {}

      generate_with_preconverted(
        original_schema_json: schema_json,
        codec_json: JSON.generate(codec),
        llm_schema: llm_schema,
        prompt: prompt
      )
    rescue JslWasiError => e
      raise SchemaConversionError, "Schema conversion failed: #{e.message}"
    end

    # Execute a roundtrip with a pre-converted schema (skips the convert step).
    #
    # @param original_schema_json [String] the original JSON Schema as a string
    # @param codec_json [String] the codec (rehydration map) as a string
    # @param llm_schema [Hash] the LLM-compatible schema (already converted)
    # @param prompt [String] the natural language prompt for the LLM
    # @return [RoundtripResult]
    def generate_with_preconverted(original_schema_json:, codec_json:, llm_schema:, prompt:)
      # Step 2: Format the request for the provider
      request = @formatter.format(prompt, llm_schema, @config)

      # Step 3: Call the LLM via consumer transport
      raw_response = @transport.execute(request)

      # Step 4: Extract content from the response
      begin
        content = @formatter.extract_content(raw_response)
      rescue ResponseParsingError
        raise
      rescue StandardError => e
        raise ResponseParsingError, "Failed to extract content: #{e.message}"
      end

      # Step 5: Rehydrate the output
      begin
        rehydrate_result = call_wasi("jsl_rehydrate", content, codec_json, original_schema_json)
      rescue JslWasiError => e
        raise RehydrationError, "Rehydration failed: #{e.message}"
      end

      rehydrated_data = rehydrate_result["data"] || {}
      warnings = (rehydrate_result["warnings"] || []).map { |w| w.is_a?(String) ? w : w.to_s }

      # Step 6: Validate against original schema
      validation_errors = validate(rehydrated_data, original_schema_json)

      RoundtripResult.new(
        data: rehydrated_data,
        raw_llm_response: JSON.parse(raw_response),
        warnings: warnings,
        validation_errors: validation_errors
      )
    end

    # Release WASM module references.
    def close
      @module = nil
      @wasm_engine = nil
    end

    private

    # Internal error for WASI call failures (not exposed to consumers).
    class JslWasiError < StandardError; end

    def validate_formatter!(formatter)
      unless formatter.respond_to?(:format) && formatter.respond_to?(:extract_content)
        raise ArgumentError,
              "formatter must respond to :format and :extract_content"
      end
    end

    def validate_transport!(transport)
      unless transport.respond_to?(:execute)
        raise ArgumentError, "transport must respond to :execute"
      end
    end

    def validate(data, schema_json)
      return [] unless JSON_SCHEMER_AVAILABLE

      begin
        schema = JSON.parse(schema_json)
        schemer = JSONSchemer.schema(schema)
        schemer.validate(data).map { |e| e["error"] }
      rescue StandardError
        []
      end
    end

    # Execute a WASI export following the JslResult protocol.
    def call_wasi(func_name, *json_args)
      wasi_ctx = Wasmtime::WasiCtxBuilder.new
                   .set_stdin_string("")
                   .inherit_stdout
                   .inherit_stderr
                   .build
      store = Wasmtime::Store.new(@wasm_engine, wasi_ctx: wasi_ctx)
      instance = Wasmtime::Linker.new(@wasm_engine, wasi: true)
                   .instantiate(store, @module)

      memory = instance.export("memory").to_memory

      # ABI version handshake
      abi_fn = instance.export("jsl_abi_version")&.to_func
      raise JslWasiError, "Missing jsl_abi_version export" unless abi_fn

      version = abi_fn.call
      unless version == EXPECTED_ABI_VERSION
        raise JslWasiError,
              "ABI version mismatch: binary=#{version}, expected=#{EXPECTED_ABI_VERSION}"
      end

      jsl_alloc = instance.export("jsl_alloc").to_func
      jsl_free = instance.export("jsl_free").to_func
      jsl_result_free = instance.export("jsl_result_free").to_func
      func = instance.export(func_name).to_func

      allocs = []
      flat_args = []
      result_ptr = nil

      begin
        json_args.each do |arg|
          bytes = arg.encode("UTF-8")
          ptr = jsl_alloc.call(bytes.bytesize)
          memory.write(ptr, bytes)
          allocs << [ptr, bytes.bytesize]
          flat_args.push(ptr, bytes.bytesize)
        end

        result_ptr = func.call(*flat_args)

        # Read JslResult (12 bytes: 3 × LE u32)
        result_bytes = memory.read(result_ptr, JSL_RESULT_SIZE)
        status, payload_ptr, payload_len = result_bytes.unpack("V3")

        payload_bytes = memory.read(payload_ptr, payload_len)
        payload = JSON.parse(payload_bytes)

        if status == STATUS_ERROR
          error_msg = payload.is_a?(Hash) ? payload["message"] || "unknown error" : payload.to_s
          error_code = payload.is_a?(Hash) ? payload["code"] || "unknown" : "unknown"
          raise JslWasiError, "[#{error_code}] #{error_msg}"
        end

        payload
      ensure
        jsl_result_free.call(result_ptr) if result_ptr
        allocs.each { |ptr, len| jsl_free.call(ptr, len) }
      end
    end

    def resolve_wasm_bytes(explicit)
      # Tier 1: Explicit path
      if explicit
        return File.binread(explicit) if File.file?(explicit)

        raise "Explicit WASM path not found: #{explicit}"
      end

      # Tier 2: Environment variable
      env = ENV["JSON_SCHEMA_LLM_WASM_PATH"]
      if env && File.file?(env)
        return File.binread(env)
      end

      # Tier 3: Repo-relative fallback (dev/CI)
      repo_path = File.join(
        __dir__, "..", "..", "..", "..",
        "target", "wasm32-wasip1", "release", "json_schema_llm_wasi.wasm"
      )
      if File.file?(repo_path)
        return File.binread(repo_path)
      end

      raise "WASM binary not found. Set JSON_SCHEMA_LLM_WASM_PATH or build the WASI target (make build-wasi)."
    end
  end
end
