"""Exception hierarchy for the jsonschema-llm Python engine."""


class EngineError(RuntimeError):
    """Base exception for unrecoverable engine failures."""


class SchemaConversionError(EngineError):
    """WASI convert operation failed."""


class RehydrationError(EngineError):
    """WASI rehydrate operation failed."""


class ResponseParsingError(EngineError):
    """Formatter couldn't parse/extract content from LLM response."""
