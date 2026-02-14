package com.jsonschema.llm;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

public record ConvertResult(
    String apiVersion,
    JsonNode schema,
    JsonNode codec,
    List<JsonNode> providerCompatErrors
) {}
