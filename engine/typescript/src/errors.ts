/**
 * Engine error hierarchy — mirrors Python engine's exceptions.py.
 */

/** Thrown when the WASI convert step fails. */
export class SchemaConversionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SchemaConversionError";
  }
}

/** Thrown when the WASI rehydrate step fails. */
export class RehydrationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RehydrationError";
  }
}

/** Thrown when the LLM response cannot be parsed. */
export class ResponseParsingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ResponseParsingError";
  }
}
