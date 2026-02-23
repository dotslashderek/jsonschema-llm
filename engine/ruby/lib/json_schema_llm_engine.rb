# frozen_string_literal: true

# json_schema_llm_engine — LLM roundtrip orchestration for Ruby.
#
# Provides the same LlmRoundtripEngine(formatter, config, transport) pattern
# as the Java, Python, and TypeScript engines.

require_relative "json_schema_llm_engine/types"
require_relative "json_schema_llm_engine/exceptions"
require_relative "json_schema_llm_engine/transport"
require_relative "json_schema_llm_engine/formatters/chat_completions"
require_relative "json_schema_llm_engine/llm_roundtrip_engine"
