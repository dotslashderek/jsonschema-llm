package com.jsonschema.llm;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

public record RehydrateResult(
    String apiVersion,
    JsonNode data,
    List<JsonNode> warnings
) {}
