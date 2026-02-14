package com.jsonschema.llm;

import com.fasterxml.jackson.annotation.JsonProperty;

public class ConvertOptions {

    public enum Target {
        @JsonProperty("openai-strict") OPENAI_STRICT,
        @JsonProperty("gemini") GEMINI,
        @JsonProperty("claude") CLAUDE
    }

    public enum Mode {
        @JsonProperty("strict") STRICT,
        @JsonProperty("permissive") PERMISSIVE
    }

    public enum PolymorphismStrategy {
        @JsonProperty("any-of") ANY_OF,
        @JsonProperty("flatten") FLATTEN
    }

    @JsonProperty("target")
    private Target target = Target.OPENAI_STRICT;

    @JsonProperty("mode")
    private Mode mode = Mode.STRICT;

    @JsonProperty("max-depth")
    private int maxDepth = 50;

    @JsonProperty("recursion-limit")
    private int recursionLimit = 3;

    @JsonProperty("polymorphism")
    private PolymorphismStrategy polymorphism = PolymorphismStrategy.ANY_OF;

    public static ConvertOptions builder() {
        return new ConvertOptions();
    }

    public ConvertOptions target(Target target) {
        this.target = target;
        return this;
    }

    public ConvertOptions mode(Mode mode) {
        this.mode = mode;
        return this;
    }

    public ConvertOptions maxDepth(int maxDepth) {
        this.maxDepth = maxDepth;
        return this;
    }

    public ConvertOptions recursionLimit(int recursionLimit) {
        this.recursionLimit = recursionLimit;
        return this;
    }

    public ConvertOptions polymorphism(PolymorphismStrategy polymorphism) {
        this.polymorphism = polymorphism;
        return this;
    }

    public Target getTarget() {
        return target;
    }

    public Mode getMode() {
        return mode;
    }

    public int getMaxDepth() {
        return maxDepth;
    }

    public int getRecursionLimit() {
        return recursionLimit;
    }

    public PolymorphismStrategy getPolymorphism() {
        return polymorphism;
    }

    public ConvertOptions build() {
        return this;
    }
}
