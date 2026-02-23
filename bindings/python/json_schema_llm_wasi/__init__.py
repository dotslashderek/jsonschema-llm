"""json-schema-llm WASI bindings — consumer-ready Python SDK.

Provides typed results, options builder, and SchemaLlmEngine facade
for zero-config WASM-powered schema conversion and rehydration.

Usage::

    from json_schema_llm_wasi import SchemaLlmEngine, ConvertOptions

    with SchemaLlmEngine() as engine:
        result = engine.convert(schema, ConvertOptions(target="openai-strict"))
        print(result.schema)
"""

from json_schema_llm_wasi.engine import JslError, SchemaLlmEngine
from json_schema_llm_wasi.types import (
    ConvertAllComponentsResult,
    ConvertOptions,
    ConvertResult,
    ExtractComponentResult,
    ListComponentsResult,
    RehydrateResult,
)

__all__ = [
    "SchemaLlmEngine",
    "JslError",
    "ConvertResult",
    "RehydrateResult",
    "ListComponentsResult",
    "ExtractComponentResult",
    "ConvertAllComponentsResult",
    "ConvertOptions",
]
