# frozen_string_literal: true

# WASI-backed wrapper for jsonschema-llm.
#
# Uses wasmtime gem to load the universal WASI binary and exposes
# convert() and rehydrate() as Ruby methods.
#
# Concurrency: Each Engine owns its own Store. NOT thread-safe.

require "wasmtime"
require "json"

module JsonSchemaLlm
  JSL_RESULT_SIZE = 12 # 3 × u32 (LE)
  STATUS_OK = 0
  STATUS_ERROR = 1

  class JslError < StandardError
    attr_reader :code, :path

    def initialize(code:, message:, path: "")
      @code = code
      @path = path
      path_str = path.empty? ? "" : " at #{path}"
      super("jsl error [#{code}]#{path_str}: #{message}")
    end
  end

  class Engine
    def initialize(wasm_path: nil)
      path = wasm_path || ENV.fetch("JSL_WASM_PATH") {
        File.join(__dir__, "..", "..",
                  "target", "wasm32-wasip1", "release", "jsonschema_llm_wasi.wasm")
      }
      @engine = Wasmtime::Engine.new
      @module = Wasmtime::Module.from_file(@engine, path)
      @linker = Wasmtime::Linker.new(@engine)
      Wasmtime::WASI::P1.add_to_linker_sync(@linker)
    end

    def convert(schema, options = {})
      schema_json = JSON.generate(schema)
      opts_json = JSON.generate(options)
      call_jsl("jsl_convert", schema_json, opts_json)
    end

    def rehydrate(data, codec, schema)
      data_json = JSON.generate(data)
      codec_json = JSON.generate(codec)
      schema_json = JSON.generate(schema)
      call_jsl("jsl_rehydrate", data_json, codec_json, schema_json)
    end

    def close
      # No persistent resources
    end

    private

    def call_jsl(func_name, *json_args)
      # Fresh store + instance per call
      wasi_config = Wasmtime::WasiConfig.new
                      .set_stdin_string("")
                      .inherit_stdout
                      .inherit_stderr
      store = Wasmtime::Store.new(@engine, wasi_p1_config: wasi_config)
      instance = @linker.instantiate(store, @module)

      memory = instance.export("memory").to_memory
      jsl_alloc = instance.export("jsl_alloc").to_func
      jsl_free = instance.export("jsl_free").to_func
      jsl_result_free = instance.export("jsl_result_free").to_func
      func = instance.export(func_name).to_func

      # Allocate and write arguments
      allocs = []
      flat_args = []
      json_args.each do |arg|
        bytes = arg.encode("UTF-8")
        ptr = jsl_alloc.call(bytes.bytesize)
        memory.write(ptr, bytes)
        allocs << [ptr, bytes.bytesize]
        flat_args.push(ptr, bytes.bytesize)
      end

      # Call function
      result_ptr = func.call(*flat_args)

      # Read JslResult (12 bytes: 3 × LE u32)
      result_bytes = memory.read(result_ptr, JSL_RESULT_SIZE)
      status, payload_ptr, payload_len = result_bytes.unpack("V3")

      # Read and parse payload
      payload_bytes = memory.read(payload_ptr, payload_len)
      payload = JSON.parse(payload_bytes)

      # Free
      jsl_result_free.call(result_ptr)
      allocs.each { |ptr, len| jsl_free.call(ptr, len) }

      if status == STATUS_ERROR
        raise JslError.new(
          code: payload["code"] || "unknown",
          message: payload["message"] || "unknown error",
          path: payload["path"] || ""
        )
      end

      payload
    end
  end
end
