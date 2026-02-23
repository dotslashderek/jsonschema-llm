//! Schema component extraction.
//!
//! Extracts a single component from a `$defs`-heavy schema by JSON Pointer
//! (RFC 6901), transitively resolving all reachable `$ref` dependencies and
//! producing a self-contained sub-schema.
//!
//! Supports `$anchor`-style fragment references (e.g., `"#stepId"`) by
//! building an anchor map via [`crate::anchor_utils::build_anchor_map`].
//!
//! ## Usage
//!
//! ```rust,no_run
//! use json_schema_llm_core::{extract_component, ExtractOptions};
//! use serde_json::json;
//!
//! let schema = json!({
//!     "$defs": {
//!         "Pet": {
//!             "type": "object",
//!             "properties": {
//!                 "name": { "type": "string" },
//!                 "tag": { "$ref": "#/$defs/Tag" }
//!             }
//!         },
//!         "Tag": { "type": "object", "properties": { "id": { "type": "integer" } } }
//!     }
//! });
//!
//! let result = extract_component(&schema, "#/$defs/Pet", &ExtractOptions::default()).unwrap();
//! assert_eq!(result.dependency_count, 1);
//! assert!(result.schema["$defs"]["Tag"].is_object());
//! ```

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::ConvertError;
use crate::schema_utils::{escape_pointer_segment, resolve_pointer, unescape_pointer_segment};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options for component extraction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "kebab-case", default)]
pub struct ExtractOptions {
    /// Maximum DFS depth for transitive dependency resolution.
    /// `None` means unbounded (default).
    pub max_depth: Option<usize>,
}

/// Result of a successful component extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractResult {
    /// The self-contained extracted schema.
    pub schema: Value,
    /// The JSON Pointer used for extraction.
    pub pointer: String,
    /// Number of unique transitive dependencies pulled in.
    pub dependency_count: usize,
    /// Pointers of transitive deps that could not be resolved locally.
    /// These `$ref` values are left as-is in the output; callers decide how to handle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_refs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Enumerate all extractable component JSON Pointers in a schema.
///
/// Walks the schema recursively, discovering components in:
/// - `$defs` (JSON Schema Draft 2019-09+)
/// - `definitions` (legacy Draft 4-7)
/// - `components/schemas` at root level only (OpenAPI Specification)
///
/// Returns a sorted, deduplicated list of JSON Pointer strings (RFC 6901).
///
/// # Notes
///
/// - OAS `components/schemas` is only detected at the document root (`#`) to
///   avoid false positives when user schemas happen to have properties named
///   `components`.
/// - Keys are RFC 6901-escaped via [`crate::schema_utils::escape_pointer_segment`].
/// - Nested `$defs` inside `$defs` entries are discovered with the correct
///   full path (e.g. `#/$defs/Outer/$defs/Inner`).
///
/// # Example
///
/// ```rust,no_run
/// use json_schema_llm_core::list_components;
/// use serde_json::json;
///
/// let schema = json!({ "$defs": { "Pet": { "type": "object" } } });
/// assert_eq!(list_components(&schema), vec!["#/$defs/Pet"]);
/// ```
pub fn list_components(schema: &Value) -> Vec<String> {
    let mut pointers: Vec<String> = Vec::new();
    collect_components(schema, "#", true, &mut pointers);
    pointers.sort();
    pointers.dedup();
    pointers
}

/// Recursive helper for [`list_components`].
///
/// Walks ALL object properties to discover `$defs`, `definitions`, and OAS
/// `components/schemas` containers, including those nested inside non-schema
/// keys (e.g., OAS `paths`). This intentionally does NOT use
/// [`SchemaFolder`](crate::schema_walker::SchemaFolder), which only traverses
/// standard JSON Schema keywords and would miss definitions in arbitrary
/// container keys.
///
/// * `node`        — current schema node
/// * `path`        — JSON Pointer prefix for this node (e.g. `"#"` or `"#/$defs/Outer"`)
/// * `is_root`     — true only when `path == "#"` (gates OAS `components/schemas` detection)
/// * `out`         — accumulator for discovered pointers
fn collect_components(node: &Value, path: &str, is_root: bool, out: &mut Vec<String>) {
    match node {
        // ------------------------------------------------------------------
        // Array: recurse into each element (handles allOf/anyOf/oneOf/items)
        // ------------------------------------------------------------------
        Value::Array(arr) => {
            for (i, elem) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", path, i);
                collect_components(elem, &child_path, false, out);
            }
        }

        // ------------------------------------------------------------------
        // Object: enumerate $defs/definitions/components/schemas and recurse
        // ------------------------------------------------------------------
        Value::Object(obj) => {
            // ---------------------------------------------------------------
            // Detect `$defs` / `definitions` map-of-components at this node
            // ---------------------------------------------------------------
            for keyword in ["$defs", "definitions"] {
                if let Some(Value::Object(defs)) = obj.get(keyword) {
                    for key in defs.keys() {
                        let escaped = escape_pointer_segment(key);
                        let ptr = format!("{}/{}/{}", path, keyword, escaped);
                        out.push(ptr.clone());
                        // Recurse into this component's schema (for nested $defs).
                        // The component itself is NOT the root — no OAS detection inside it.
                        collect_components(&defs[key], &ptr, false, out);
                    }
                }
            }

            // ---------------------------------------------------------------
            // Detect OAS `components/schemas` — ONLY at the document root
            // ---------------------------------------------------------------
            if is_root {
                if let Some(Value::Object(components)) = obj.get("components") {
                    if let Some(Value::Object(schemas)) = components.get("schemas") {
                        for key in schemas.keys() {
                            let escaped = escape_pointer_segment(key);
                            let ptr = format!("#/components/schemas/{}", escaped);
                            out.push(ptr.clone());
                            // Recurse for nested $defs inside OAS component schemas.
                            collect_components(&schemas[key], &ptr, false, out);
                        }
                    }
                }
            }

            // ---------------------------------------------------------------
            // Recurse into all remaining object values.
            // Skip keys already processed above to avoid double-counting.
            // ---------------------------------------------------------------
            for (key, val) in obj {
                if key == "$defs" || key == "definitions" {
                    // Already enumerated component keys above; recursion into each
                    // component's body is handled in the enumeration loop above.
                    continue;
                }
                if is_root && key == "components" {
                    // Already handled OAS components/schemas above.
                    continue;
                }
                let child_path = format!("{}/{}", path, escape_pointer_segment(key));
                collect_components(val, &child_path, false, out);
            }
        }

        // All other Value variants (String, Number, Bool, Null) cannot contain $defs.
        _ => {}
    }
}

/// Extract a single component from a schema by JSON Pointer.
///
/// Resolves the target pointer, performs a DFS transitive closure over all
/// reachable `$ref` dependencies, tree-shakes the result to include only
/// discovered definitions, and rewrites `$ref` pointers to the new namespace.
///
/// # Arguments
///
/// * `schema` - The source JSON Schema document
/// * `pointer` - RFC 6901 JSON Pointer to the component (e.g., `#/$defs/Pet`
///   or `#/components/schemas/Pet`)
/// * `options` - Extraction options (depth limit)
///
/// # Returns
///
/// An [`ExtractResult`] with the self-contained schema, or a [`ConvertError`]
/// if the target pointer does not exist or a reachable `$ref` is external.
///
/// Unresolvable **transitive** deps are reported in [`ExtractResult::missing_refs`]
/// and left as dangling `$ref` values in the output (soft-fail).
pub fn extract_component(
    schema: &Value,
    pointer: &str,
    options: &ExtractOptions,
) -> Result<ExtractResult, ConvertError> {
    // Phase 1: Resolve the target pointer (hard error if missing).
    let target = resolve_pointer(schema, pointer).ok_or_else(|| ConvertError::UnresolvableRef {
        path: pointer.to_string(),
        reference: pointer.to_string(),
    })?;
    let target = target.clone();

    // Phase 1b: Build resolver engine for $anchor/$id resolution.
    let resolver = crate::resolver::ResolverEngine::new(schema)?;

    // Phase 2: Transitive closure — DFS to collect all reachable deps.
    let starting_base = resolver.parent_base_uri_for_pointer(schema, pointer);
    let mut ctx = DfsCtx {
        root_schema: schema,
        max_depth: options.max_depth.unwrap_or(usize::MAX),
        visited: HashSet::new(),
        deps: BTreeMap::new(),
        missing_refs: Vec::new(),
        resolver: &resolver,
        base_uri: starting_base,
    };
    ctx.visited.insert(pointer.to_string());

    collect_deps(&target, pointer, 0, &mut ctx)?;

    let DfsCtx {
        deps,
        mut missing_refs,
        ..
    } = ctx;

    let dependency_count = deps.len();

    // Phase 3: Tree-shaking — build new root + $defs.
    // The target node becomes the root schema; deps become $defs.
    // Build a ref-rewrite map: old pointer → new "#/$defs/<key>" reference.
    // IMPORTANT: re-escape the key for use in a JSON Pointer.
    let rewrite_map: BTreeMap<String, String> = deps
        .iter()
        .map(|(ptr, (key, _, _))| {
            let escaped_key = escape_pointer_segment(key);
            (ptr.clone(), format!("#/$defs/{}", escaped_key))
        })
        .collect();

    // Phase 4: Rewrite refs in the target node and all dep nodes.
    let mut root = rewrite_refs_aware(target, resolver.base_uri(), &resolver, &rewrite_map);

    // Phase 5: Assemble $defs if there are any deps.
    if !deps.is_empty() {
        let mut defs_map = Map::new();
        for (_ptr, (key, value, dep_base_uri)) in deps {
            let rewritten = rewrite_refs_aware(value, &dep_base_uri, &resolver, &rewrite_map);
            defs_map.insert(key, rewritten);
        }
        if let Value::Object(ref mut obj) = root {
            obj.insert("$defs".to_string(), Value::Object(defs_map));
        }
    }

    // Sort missing_refs for deterministic output.
    missing_refs.sort();
    missing_refs.dedup();

    Ok(ExtractResult {
        schema: root,
        pointer: pointer.to_string(),
        dependency_count,
        missing_refs,
    })
}

// ---------------------------------------------------------------------------
// DFS context
// ---------------------------------------------------------------------------

/// Mutable + readonly context threaded through the DFS traversal.
struct DfsCtx<'a> {
    root_schema: &'a Value,
    max_depth: usize,
    visited: HashSet<String>,
    /// pointer → (key, resolved_value). `BTreeMap` for deterministic output.
    deps: BTreeMap<String, (String, Value, url::Url)>,
    missing_refs: Vec<String>,
    /// Centralized resolver engine for $ref resolution.
    resolver: &'a crate::resolver::ResolverEngine,
    /// Current base URI for $id scoping during DFS.
    base_uri: url::Url,
}

// ---------------------------------------------------------------------------
// DFS transitive closure
// ---------------------------------------------------------------------------

/// DFS walker: collects all reachable `$ref` dependencies from `node`.
///
/// For each `$ref` encountered:
/// - External refs (not starting with `#`) → hard error
/// - Anchor-style refs (`#Foo`) → hard error
/// - Already visited → skip (cycle break), but still traverse siblings
/// - Unresolvable local ref → record in `missing_refs`, leave as-is (soft-fail)
/// - Resolvable → compute key, add to `deps`, recurse
fn collect_deps(
    node: &Value,
    current_path: &str,
    depth: usize,
    ctx: &mut DfsCtx<'_>,
) -> Result<(), ConvertError> {
    if depth > ctx.max_depth {
        return Err(ConvertError::RecursionDepthExceeded {
            path: current_path.to_string(),
            max_depth: ctx.max_depth,
        });
    }

    match node {
        Value::Object(obj) => {
            // Save base URI — $id scoping is lexical (per-subtree), not global.
            let saved_base = ctx.base_uri.clone();

            // Track $id for base URI scoping.
            if let Some(id_val) = obj.get("$id").and_then(Value::as_str) {
                if let Ok(new_base) = ctx.base_uri.join(id_val) {
                    ctx.base_uri = new_base;
                }
            }

            if let Some(ref_val) = obj.get("$ref").and_then(Value::as_str) {
                // Resolve via centralized ResolverEngine.
                let effective_ref = match ctx.resolver.resolve(ref_val, &ctx.base_uri) {
                    crate::resolver::ResolvedRef::Pointer(p) => p,
                    crate::resolver::ResolvedRef::Unresolvable(_) => {
                        // Soft-fail: external or unresolvable anchor refs are
                        // recorded as missing and left as-is.
                        ctx.missing_refs.push(ref_val.to_string());
                        ctx.base_uri = saved_base;
                        return Ok(());
                    }
                };

                let ref_val = effective_ref.as_str();

                // Skip if already visited (cycle break) — but still traverse siblings below.
                let already_visited = ctx.visited.contains(ref_val);

                if !already_visited {
                    // Attempt to resolve.
                    match resolve_pointer(ctx.root_schema, ref_val) {
                        None => {
                            // Soft-fail: record as missing, leave $ref dangling.
                            ctx.missing_refs.push(ref_val.to_string());
                        }
                        Some(resolved) => {
                            let key = pointer_to_key(ref_val, &ctx.deps);
                            let resolved_clone = resolved.clone();
                            ctx.visited.insert(ref_val.to_string());

                            let target_base_uri = ctx
                                .resolver
                                .parent_base_uri_for_pointer(ctx.root_schema, ref_val);

                            ctx.deps.insert(
                                ref_val.to_string(),
                                (key, resolved_clone.clone(), target_base_uri.clone()),
                            );

                            let saved_base = ctx.base_uri.clone();
                            ctx.base_uri = target_base_uri;

                            // Only increment depth for $ref hops (not AST traversal).
                            collect_deps(&resolved_clone, ref_val, depth + 1, ctx)?;

                            ctx.base_uri = saved_base;
                        }
                    }
                }

                // Always continue DFS into siblings (regardless of cycle detection).
                // JSON Schema Draft 2019-09+ allows schemas alongside $ref.
                for (key, val) in obj {
                    if key == "$ref" {
                        continue;
                    }
                    let child_path = format!("{}/{}", current_path, key);
                    // Do NOT increment depth here — sibling traversal is not a ref hop.
                    collect_deps(val, &child_path, depth, ctx)?;
                }
                ctx.base_uri = saved_base;
                return Ok(());
            }

            // No $ref — recurse into all values (depth unchanged: not a ref hop).
            for (key, val) in obj {
                let child_path = format!("{}/{}", current_path, key);
                collect_deps(val, &child_path, depth, ctx)?;
            }
            ctx.base_uri = saved_base;
        }
        Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", current_path, i);
                // Depth unchanged: array traversal is not a ref hop.
                collect_deps(val, &child_path, depth, ctx)?;
            }
        }
        _ => {}
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Key computation
// ---------------------------------------------------------------------------

/// Compute a canonical, collision-safe, unique key for a JSON Pointer.
///
/// Strategy:
/// 1. Always try the **last segment** first (e.g. `#/components/schemas/Tag` → `"Tag"`)
/// 2. If that key is **already taken** by a different pointer, fall back to the
///    full-path sanitized form (join all segments with `_`)
/// 3. If the fallback ALSO collides (rare path ambiguity), append `_N` (N=2,3,...)
///    until unique.
///
/// All segment strings are RFC 6901 unescaped (for human-readable key names).
/// Callers must escape keys when building JSON Pointer strings.
fn pointer_to_key(pointer: &str, deps: &BTreeMap<String, (String, Value, url::Url)>) -> String {
    let stripped = pointer.strip_prefix('#').unwrap_or(pointer);
    let segments: Vec<&str> = stripped.split('/').filter(|s| !s.is_empty()).collect();

    let last = segments
        .last()
        .map(|s| unescape_pointer_segment(s).into_owned())
        .unwrap_or_else(|| "root".to_string());

    // Check for collision: key taken by a DIFFERENT pointer?
    let key_exists = |candidate: &str| {
        deps.iter()
            .any(|(ptr, (k, _, _))| k == candidate && ptr != pointer)
    };

    if !key_exists(&last) {
        return last;
    }

    // Fallback: full-path join.
    let joined = segments
        .iter()
        .map(|s| unescape_pointer_segment(s).into_owned())
        .collect::<Vec<_>>()
        .join("_");

    if !key_exists(&joined) {
        return joined;
    }

    // Last resort: append numeric suffix until unique.
    let mut n = 2usize;
    loop {
        let candidate = format!("{}_{}", joined, n);
        if !key_exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

// `unescape_segment` was removed — use `schema_utils::unescape_pointer_segment` instead.
// The local function was an exact duplicate of the schema_utils version.

// ---------------------------------------------------------------------------
// Ref rewriting
// ---------------------------------------------------------------------------

/// Walk a schema value and rewrite all `$ref` strings using the provided map,
/// tracking the base URI to accurately resolve anchors and relative paths.
fn rewrite_refs_aware(
    value: Value,
    current_base: &url::Url,
    resolver: &crate::resolver::ResolverEngine,
    pointer_rewrite_map: &std::collections::BTreeMap<String, String>,
) -> Value {
    let mut obj = match value {
        Value::Object(o) => o,
        Value::Array(arr) => {
            return Value::Array(
                arr.into_iter()
                    .map(|v| rewrite_refs_aware(v, current_base, resolver, pointer_rewrite_map))
                    .collect(),
            );
        }
        other => return other,
    };

    let mut scoped_base = current_base.clone();
    if let Some(id_val) = obj.get("$id").and_then(Value::as_str) {
        if let Ok(new_base) = current_base.join(id_val) {
            scoped_base = new_base;
        }
    }

    if let Some(Value::String(ref_str)) = obj.get("$ref") {
        if let crate::resolver::ResolvedRef::Pointer(target_ptr) =
            resolver.resolve(ref_str, &scoped_base)
        {
            if let Some(new_ref) = pointer_rewrite_map.get(&target_ptr) {
                obj.insert("$ref".to_string(), Value::String(new_ref.clone()));
            }
        }
    }

    let rewritten = obj
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                rewrite_refs_aware(v, &scoped_base, resolver, pointer_rewrite_map),
            )
        })
        .collect();
    Value::Object(rewritten)
}

// ---------------------------------------------------------------------------
// DependencyGraph — batch-optimized extraction (#190)
// ---------------------------------------------------------------------------

/// Pre-computed dependency graph for batch component extraction.
///
/// Instead of running a full DFS per component (O(N × M)), this struct
/// builds a global adjacency list in a single O(M) pass, then slices
/// per-component sub-graphs via fast in-memory DFS (O(N × avg_deps)).
///
/// # Usage
///
/// ```rust,no_run
/// use json_schema_llm_core::{DependencyGraph, ExtractOptions};
/// use serde_json::json;
///
/// let schema = json!({
///     "$defs": {
///         "A": { "type": "object", "properties": { "b": { "$ref": "#/$defs/B" } } },
///         "B": { "type": "integer" }
///     }
/// });
/// let graph = DependencyGraph::build(&schema, &ExtractOptions::default()).unwrap();
/// let result = graph.extract("#/$defs/A", &ExtractOptions::default()).unwrap();
/// assert_eq!(result.dependency_count, 1);
/// ```
pub struct DependencyGraph<'a> {
    /// Direct adjacency: each pointer → ordered list of pointers it directly `$ref`s.
    /// Uses `Vec` (not `BTreeSet`) to preserve DFS discovery order for deterministic
    /// `pointer_to_key` output.
    edges: BTreeMap<String, Vec<String>>,
    /// Borrowed node values + computed base URIs for each discovered pointer.
    nodes: BTreeMap<String, (&'a Value, url::Url)>,
    /// Per-pointer missing (unresolvable) refs discovered during build.
    global_missing: BTreeMap<String, Vec<String>>,
    /// Shared resolver engine (built once for the entire schema).
    resolver: crate::resolver::ResolverEngine,
}

impl<'a> DependencyGraph<'a> {
    /// Build the global dependency graph for all components in a single pass.
    ///
    /// Discovers all component pointers via [`list_components`], then runs a
    /// DFS from each root to collect direct `$ref` edges and resolve node values.
    /// Nodes already visited by a previous root's DFS are skipped (shared dep
    /// short-circuit).
    pub fn build(
        schema: &'a Value,
        _options: &ExtractOptions,
    ) -> Result<DependencyGraph<'a>, ConvertError> {
        let resolver = crate::resolver::ResolverEngine::new(schema)?;
        let pointers = list_components(schema);

        let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut nodes: BTreeMap<String, (&'a Value, url::Url)> = BTreeMap::new();
        let mut global_missing: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut globally_visited: HashSet<String> = HashSet::new();

        for pointer in &pointers {
            // Resolve the component node itself.
            let target = match resolve_pointer(schema, pointer) {
                Some(v) => v,
                None => continue, // Skip unresolvable roots — will error at extract time.
            };

            let base_uri = resolver.parent_base_uri_for_pointer(schema, pointer);
            nodes
                .entry(pointer.clone())
                .or_insert((target, base_uri.clone()));

            // Run DFS to discover direct edges from this subtree.
            if globally_visited.insert(pointer.clone()) {
                let mut edge_ctx = EdgeBuildCtx {
                    root_schema: schema,
                    visited: &mut globally_visited,
                    edges: &mut edges,
                    nodes: &mut nodes,
                    global_missing: &mut global_missing,
                    resolver: &resolver,
                    base_uri,
                };
                collect_direct_edges(target, pointer, pointer, &mut edge_ctx)?;
            }
        }

        Ok(DependencyGraph {
            edges,
            nodes,
            global_missing,
            resolver,
        })
    }

    /// Extract a self-contained sub-schema for a single component pointer.
    ///
    /// Computes the transitive closure by walking the pre-built adjacency list
    /// (no schema traversal), then assembles the result using the same ref
    /// rewriting logic as [`extract_component`].
    pub fn extract(
        &self,
        pointer: &str,
        options: &ExtractOptions,
    ) -> Result<ExtractResult, ConvertError> {
        // Phase 1: Resolve the target node (hard error if missing from the graph).
        let (target, _) = self
            .nodes
            .get(pointer)
            .ok_or_else(|| ConvertError::UnresolvableRef {
                path: pointer.to_string(),
                reference: pointer.to_string(),
            })?;
        let target = (*target).clone();

        // Phase 2: Compute transitive closure via DFS on the adjacency list.
        let max_depth = options.max_depth.unwrap_or(usize::MAX);
        let mut closure: Vec<String> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(pointer.to_string());
        self.closure_dfs(pointer, 0, max_depth, pointer, &mut visited, &mut closure)?;

        // Phase 3: Build deps BTreeMap in DFS discovery order (matching extract_component).
        let mut deps: BTreeMap<String, (String, Value, url::Url)> = BTreeMap::new();
        for dep_ptr in &closure {
            if let Some((val, base)) = self.nodes.get(dep_ptr.as_str()) {
                let key = pointer_to_key(dep_ptr, &deps);
                deps.insert(dep_ptr.clone(), (key, (*val).clone(), base.clone()));
            }
        }

        let dependency_count = deps.len();

        // Collect missing refs for this component's reachable subgraph.
        let mut missing_refs: Vec<String> = Vec::new();
        // Add missing refs from the root pointer itself.
        if let Some(root_missing) = self.global_missing.get(pointer) {
            missing_refs.extend(root_missing.iter().cloned());
        }
        // Add missing refs from all transitive deps.
        for dep_ptr in &closure {
            if let Some(dep_missing) = self.global_missing.get(dep_ptr.as_str()) {
                missing_refs.extend(dep_missing.iter().cloned());
            }
        }

        // Phase 4: Build rewrite map.
        let rewrite_map: BTreeMap<String, String> = deps
            .iter()
            .map(|(ptr, (key, _, _))| {
                let escaped_key = escape_pointer_segment(key);
                (ptr.clone(), format!("#/$defs/{}", escaped_key))
            })
            .collect();

        // Phase 5: Rewrite refs in the target node and all dep nodes.
        let mut root = rewrite_refs_aware(
            target,
            self.resolver.base_uri(),
            &self.resolver,
            &rewrite_map,
        );

        // Phase 6: Assemble $defs.
        if !deps.is_empty() {
            let mut defs_map = serde_json::Map::new();
            for (_ptr, (key, value, dep_base_uri)) in deps {
                let rewritten =
                    rewrite_refs_aware(value, &dep_base_uri, &self.resolver, &rewrite_map);
                defs_map.insert(key, rewritten);
            }
            if let Value::Object(ref mut obj) = root {
                obj.insert("$defs".to_string(), Value::Object(defs_map));
            }
        }

        missing_refs.sort();
        missing_refs.dedup();

        Ok(ExtractResult {
            schema: root,
            pointer: pointer.to_string(),
            dependency_count,
            missing_refs,
        })
    }

    /// DFS over the adjacency list to compute the transitive closure for a root.
    fn closure_dfs(
        &self,
        current: &str,
        depth: usize,
        max_depth: usize,
        root_path: &str,
        visited: &mut HashSet<String>,
        closure: &mut Vec<String>,
    ) -> Result<(), ConvertError> {
        if let Some(neighbors) = self.edges.get(current) {
            for neighbor in neighbors {
                if visited.insert(neighbor.clone()) {
                    // Check depth: only $ref hops count.
                    if depth + 1 > max_depth {
                        return Err(ConvertError::RecursionDepthExceeded {
                            path: root_path.to_string(),
                            max_depth,
                        });
                    }
                    closure.push(neighbor.clone());
                    self.closure_dfs(neighbor, depth + 1, max_depth, root_path, visited, closure)?;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Edge-building DFS (single-pass adjacency construction)
// ---------------------------------------------------------------------------

/// Mutable context for the edge-building pass.
struct EdgeBuildCtx<'a, 'b> {
    root_schema: &'a Value,
    visited: &'b mut HashSet<String>,
    edges: &'b mut BTreeMap<String, Vec<String>>,
    nodes: &'b mut BTreeMap<String, (&'a Value, url::Url)>,
    global_missing: &'b mut BTreeMap<String, Vec<String>>,
    resolver: &'b crate::resolver::ResolverEngine,
    base_uri: url::Url,
}

/// DFS walker for building the global adjacency list.
///
/// Similar to `collect_deps`, but records direct edges (`source_root → target`)
/// instead of accumulating a per-component deps map. Resolves each `$ref`,
/// stores the resolved node in `nodes`, and adds the edge.
fn collect_direct_edges<'a>(
    node: &'a Value,
    current_path: &str,
    source_root: &str,
    ctx: &mut EdgeBuildCtx<'a, '_>,
) -> Result<(), ConvertError> {
    match node {
        Value::Object(obj) => {
            let saved_base = ctx.base_uri.clone();

            // Track $id for base URI scoping.
            if let Some(id_val) = obj.get("$id").and_then(Value::as_str) {
                if let Ok(new_base) = ctx.base_uri.join(id_val) {
                    ctx.base_uri = new_base;
                }
            }

            if let Some(ref_val) = obj.get("$ref").and_then(Value::as_str) {
                // Resolve via centralized ResolverEngine.
                let effective_ref = match ctx.resolver.resolve(ref_val, &ctx.base_uri) {
                    crate::resolver::ResolvedRef::Pointer(p) => p,
                    crate::resolver::ResolvedRef::Unresolvable(_) => {
                        ctx.global_missing
                            .entry(source_root.to_string())
                            .or_default()
                            .push(ref_val.to_string());
                        ctx.base_uri = saved_base;
                        return Ok(());
                    }
                };

                let ref_str = effective_ref.as_str();

                if !ctx.visited.contains(ref_str) {
                    match resolve_pointer(ctx.root_schema, ref_str) {
                        None => {
                            ctx.global_missing
                                .entry(source_root.to_string())
                                .or_default()
                                .push(ref_str.to_string());
                        }
                        Some(resolved) => {
                            // Record the edge: source_root → ref_str
                            ctx.edges
                                .entry(source_root.to_string())
                                .or_default()
                                .push(ref_str.to_string());

                            // Store the resolved node.
                            let target_base = ctx
                                .resolver
                                .parent_base_uri_for_pointer(ctx.root_schema, ref_str);
                            ctx.nodes
                                .entry(ref_str.to_string())
                                .or_insert((resolved, target_base.clone()));

                            // Mark visited and recurse into the resolved node.
                            ctx.visited.insert(ref_str.to_string());
                            let saved_source_base = ctx.base_uri.clone();
                            ctx.base_uri = target_base;
                            collect_direct_edges(resolved, ref_str, ref_str, ctx)?;
                            ctx.base_uri = saved_source_base;
                        }
                    }
                } else {
                    // Already visited — just record the edge (for closure computation).
                    ctx.edges
                        .entry(source_root.to_string())
                        .or_default()
                        .push(ref_str.to_string());
                }

                // Continue DFS into siblings (JSON Schema 2019-09+ allows schemas alongside $ref).
                for (key, val) in obj {
                    if key == "$ref" {
                        continue;
                    }
                    let child_path = format!("{}/{}", current_path, key);
                    collect_direct_edges(val, &child_path, source_root, ctx)?;
                }
                ctx.base_uri = saved_base;
                return Ok(());
            }

            // No $ref — recurse into all values.
            for (key, val) in obj {
                let child_path = format!("{}/{}", current_path, key);
                collect_direct_edges(val, &child_path, source_root, ctx)?;
            }
            ctx.base_uri = saved_base;
        }
        Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", current_path, i);
                collect_direct_edges(val, &child_path, source_root, ctx)?;
            }
        }
        _ => {}
    }

    Ok(())
}

// ===========================================================================
// Tests (TDD — written before implementation)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConvertOptions;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn opts() -> ExtractOptions {
        ExtractOptions::default()
    }

    // -----------------------------------------------------------------------
    // AC1: extract_component() produces valid, self-contained JSON Schema
    // -----------------------------------------------------------------------

    #[test]
    fn test_simple_extract_no_deps() {
        let schema = json!({
            "$defs": {
                "Foo": { "type": "string", "minLength": 1 },
                "Bar": { "type": "integer" }
            }
        });

        let result = extract_component(&schema, "#/$defs/Foo", &opts()).unwrap();

        assert_eq!(result.schema, json!({ "type": "string", "minLength": 1 }));
        assert_eq!(result.dependency_count, 0);
        assert!(result.missing_refs.is_empty());
        assert!(
            result.schema.get("$defs").is_none(),
            "empty $defs should be omitted"
        );
    }

    #[test]
    fn test_extract_with_direct_dep() {
        let schema = json!({
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "b": { "$ref": "#/$defs/B" }
                    }
                },
                "B": { "type": "integer" }
            }
        });

        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        assert_eq!(result.dependency_count, 1);
        assert!(result.schema["$defs"]["B"].is_object());
        assert_eq!(result.schema["$defs"]["B"], json!({ "type": "integer" }));
        // $ref should be rewritten
        assert_eq!(result.schema["properties"]["b"]["$ref"], json!("#/$defs/B"));
        assert!(result.missing_refs.is_empty());
    }

    #[test]
    fn test_transitive_deps() {
        let schema = json!({
            "$defs": {
                "A": { "type": "object", "properties": { "b": { "$ref": "#/$defs/B" } } },
                "B": { "type": "object", "properties": { "c": { "$ref": "#/$defs/C" } } },
                "C": { "type": "string" }
            }
        });

        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        assert_eq!(result.dependency_count, 2);
        assert!(result.schema["$defs"]["B"].is_object());
        assert!(result.schema["$defs"]["C"].is_object());
        assert!(result.missing_refs.is_empty());
    }

    #[test]
    fn test_deep_nesting_5_levels() {
        let schema = json!({
            "$defs": {
                "A": { "$ref": "#/$defs/B" },
                "B": { "$ref": "#/$defs/C" },
                "C": { "$ref": "#/$defs/D" },
                "D": { "$ref": "#/$defs/E" },
                "E": { "$ref": "#/$defs/F" },
                "F": { "type": "string" }
            }
        });

        // Extract A — should resolve chain B→C→D→E→F
        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();
        // A itself just contains $ref to B (as root), so deps should include B,C,D,E,F
        assert_eq!(result.dependency_count, 5);
        assert!(result.missing_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // AC3: Circular refs handled without panic or infinite loop
    // -----------------------------------------------------------------------

    #[test]
    fn test_circular_refs_no_panic() {
        let schema = json!({
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "b": { "$ref": "#/$defs/B" }
                    }
                },
                "B": {
                    "type": "object",
                    "properties": {
                        "a": { "$ref": "#/$defs/A" }
                    }
                }
            }
        });

        // Must not panic or loop forever
        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        // Both A and B should be discovered
        assert!(result.dependency_count >= 1);
        assert!(result.missing_refs.is_empty());
    }

    #[test]
    fn test_self_referencing_no_panic() {
        let schema = json!({
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "child": { "$ref": "#/$defs/Node" }
                    }
                }
            }
        });

        let result = extract_component(&schema, "#/$defs/Node", &opts()).unwrap();
        // Node refs itself — visited set should break the cycle
        assert!(result.missing_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // AC4: External $ref → ConvertError::UnsupportedFeature
    // -----------------------------------------------------------------------

    #[test]
    fn test_external_ref_soft_fail() {
        let schema = json!({
            "$defs": {
                "Pet": {
                    "type": "object",
                    "properties": {
                        "tag": { "$ref": "http://example.com/schemas/Tag" }
                    }
                }
            }
        });

        let result = extract_component(&schema, "#/$defs/Pet", &opts()).unwrap();
        assert!(
            result
                .missing_refs
                .iter()
                .any(|r| r.contains("example.com")),
            "expected external ref in missing_refs; got: {:?}",
            result.missing_refs
        );
    }

    // -----------------------------------------------------------------------
    // Target pointer not found → hard error
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_pointer_error() {
        let schema = json!({ "$defs": { "Foo": { "type": "string" } } });

        let err = extract_component(&schema, "#/$defs/DoesNotExist", &opts()).unwrap_err();
        match err {
            ConvertError::UnresolvableRef { reference, .. } => {
                assert!(reference.contains("DoesNotExist"), "got: {}", reference);
            }
            other => panic!("expected UnresolvableRef, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Soft-fail: transitive dep missing → missing_refs, not error
    // -----------------------------------------------------------------------

    #[test]
    fn test_partial_extraction_missing_dep() {
        let schema = json!({
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "b": { "$ref": "#/$defs/B" }
                    }
                }
                // B is intentionally missing
            }
        });

        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        // A should be extracted without error
        assert!(result.schema["properties"]["b"]["$ref"].is_string());
        // B should appear in missing_refs
        assert_eq!(result.missing_refs, vec!["#/$defs/B"]);
        // dependency_count counts only resolved deps
        assert_eq!(result.dependency_count, 0);
    }

    // -----------------------------------------------------------------------
    // OAS-style path: #/components/schemas/...
    // -----------------------------------------------------------------------

    #[test]
    fn test_oas_style_path() {
        let schema = json!({
            "components": {
                "schemas": {
                    "Pet": {
                        "type": "object",
                        "properties": {
                            "tag": { "$ref": "#/components/schemas/Tag" }
                        }
                    },
                    "Tag": { "type": "object", "properties": { "id": { "type": "integer" } } }
                }
            }
        });

        let result = extract_component(&schema, "#/components/schemas/Pet", &opts()).unwrap();

        assert_eq!(result.dependency_count, 1);
        // Tag should be in $defs
        assert!(result.schema["$defs"]["Tag"].is_object());
        // $ref should be rewritten to #/$defs/Tag
        assert_eq!(
            result.schema["properties"]["tag"]["$ref"],
            json!("#/$defs/Tag")
        );
        assert!(result.missing_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Deterministic output
    // -----------------------------------------------------------------------

    #[test]
    fn test_deterministic_output() {
        let schema = json!({
            "$defs": {
                "A": {
                    "anyOf": [
                        { "$ref": "#/$defs/B" },
                        { "$ref": "#/$defs/C" },
                        { "$ref": "#/$defs/D" }
                    ]
                },
                "B": { "type": "integer" },
                "C": { "type": "string" },
                "D": { "type": "boolean" }
            }
        });

        // Run 3 times, results must be identical
        let r1 = extract_component(&schema, "#/$defs/A", &opts()).unwrap();
        let r2 = extract_component(&schema, "#/$defs/A", &opts()).unwrap();
        let r3 = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        let s1 = serde_json::to_string(&r1.schema).unwrap();
        let s2 = serde_json::to_string(&r2.schema).unwrap();
        let s3 = serde_json::to_string(&r3.schema).unwrap();

        assert_eq!(s1, s2, "round 1 vs 2 differ");
        assert_eq!(s2, s3, "round 2 vs 3 differ");

        // $defs keys must be sorted (B < C < D)
        let defs = r1.schema["$defs"].as_object().unwrap();
        let keys: Vec<&str> = defs.keys().map(String::as_str).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "$defs keys not sorted");
    }

    // -----------------------------------------------------------------------
    // AC2: Extracted schema can be passed to convert()
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // list_components() — TDD gate tests (written before implementation)
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_components_empty_schema() {
        let schema = json!({});
        let result = list_components(&schema);
        assert!(result.is_empty(), "empty schema should return empty vec");
    }

    #[test]
    fn test_list_components_no_defs() {
        let schema = json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        let result = list_components(&schema);
        assert!(
            result.is_empty(),
            "schema without $defs should return empty vec"
        );
    }

    #[test]
    fn test_list_components_defs_five_entries() {
        let schema = json!({
            "$defs": {
                "A": { "type": "string" },
                "B": { "type": "integer" },
                "C": { "type": "boolean" },
                "D": { "type": "number" },
                "E": { "type": "null" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 5, "should enumerate all 5 $defs entries");
        assert!(result.contains(&"#/$defs/A".to_string()));
        assert!(result.contains(&"#/$defs/B".to_string()));
        assert!(result.contains(&"#/$defs/C".to_string()));
        assert!(result.contains(&"#/$defs/D".to_string()));
        assert!(result.contains(&"#/$defs/E".to_string()));
    }

    #[test]
    fn test_list_components_output_sorted() {
        let schema = json!({
            "$defs": {
                "Zebra": { "type": "string" },
                "Apple": { "type": "integer" },
                "Mango": { "type": "boolean" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 3);
        let mut expected = result.clone();
        expected.sort();
        assert_eq!(result, expected, "output must be sorted");
    }

    #[test]
    fn test_list_components_deterministic() {
        let schema = json!({
            "$defs": {
                "X": { "type": "string" },
                "Y": { "type": "integer" }
            }
        });
        let r1 = list_components(&schema);
        let r2 = list_components(&schema);
        let r3 = list_components(&schema);
        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
    }

    #[test]
    fn test_list_components_legacy_definitions() {
        // Pre-Draft-2019 `definitions` key should be enumerated
        let schema = json!({
            "definitions": {
                "Foo": { "type": "string" },
                "Bar": { "type": "integer" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"#/definitions/Foo".to_string()));
        assert!(result.contains(&"#/definitions/Bar".to_string()));
    }

    #[test]
    fn test_list_components_oas_components_schemas() {
        // OAS `components/schemas` at root level
        let schema = json!({
            "components": {
                "schemas": {
                    "Pet": { "type": "object" },
                    "Tag": { "type": "object" }
                }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"#/components/schemas/Pet".to_string()));
        assert!(result.contains(&"#/components/schemas/Tag".to_string()));
    }

    #[test]
    fn test_list_components_mixed_defs_and_oas() {
        let schema = json!({
            "$defs": { "LocalType": { "type": "string" } },
            "components": {
                "schemas": { "OasType": { "type": "object" } }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"#/$defs/LocalType".to_string()));
        assert!(result.contains(&"#/components/schemas/OasType".to_string()));
    }

    #[test]
    fn test_list_components_defs_inside_all_of_array() {
        // Regression for array traversal bug: $defs nested inside allOf entries
        // must be discovered even though allOf is a Value::Array.
        let schema = json!({
            "allOf": [
                {
                    "$defs": {
                        "InnerA": { "type": "string" }
                    }
                },
                {
                    "anyOf": [
                        {
                            "definitions": {
                                "DeepDefined": { "type": "integer" }
                            }
                        }
                    ]
                }
            ]
        });
        let result = list_components(&schema);
        assert!(
            result.iter().any(|p| p.contains("InnerA")),
            "InnerA (nested in allOf[0]/$defs) should be discovered; got: {:?}",
            result
        );
        assert!(
            result.iter().any(|p| p.contains("DeepDefined")),
            "DeepDefined (nested in allOf[1]/anyOf[0]/definitions) should be discovered; got: {:?}",
            result
        );
    }

    #[test]
    fn test_list_components_nested_defs_inside_defs() {
        // $defs entries that themselves contain nested $defs
        let schema = json!({
            "$defs": {
                "Outer": {
                    "type": "object",
                    "$defs": {
                        "Inner": { "type": "string" }
                    }
                }
            }
        });
        let result = list_components(&schema);
        // Should find both Outer and Outer/Inner
        assert!(
            result.contains(&"#/$defs/Outer".to_string()),
            "missing Outer; got: {:?}",
            result
        );
        assert!(
            result.contains(&"#/$defs/Outer/$defs/Inner".to_string()),
            "missing Outer/$defs/Inner; got: {:?}",
            result
        );
    }

    #[test]
    fn test_list_components_special_char_key_escaped() {
        // Keys with `/` must be RFC 6901 escaped in the returned pointer
        let schema = json!({
            "$defs": {
                "user/name": { "type": "string" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "#/$defs/user~1name");
    }

    #[test]
    fn test_list_components_oas_not_at_nested_level() {
        // components/schemas nested inside properties should NOT be enumerated
        let schema = json!({
            "properties": {
                "components": {
                    "properties": {
                        "schemas": {
                            "properties": {
                                "Thing": { "type": "string" }
                            }
                        }
                    }
                }
            }
        });
        let result = list_components(&schema);
        assert!(
            result.is_empty(),
            "components/schemas buried in properties should not be listed; got: {:?}",
            result
        );
    }

    #[test]
    fn test_extracted_passes_convert() {
        let schema = json!({
            "$defs": {
                "Pet": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "tag": { "$ref": "#/$defs/Tag" }
                    },
                    "required": ["name"]
                },
                "Tag": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" }
                    }
                }
            }
        });

        let result = extract_component(&schema, "#/$defs/Pet", &opts()).unwrap();
        let convert_result = crate::convert(&result.schema, &ConvertOptions::default());
        assert!(
            convert_result.is_ok(),
            "convert() failed on extracted schema: {:?}",
            convert_result.unwrap_err()
        );
    }

    // -----------------------------------------------------------------------
    // Root-level extraction (#)
    // -----------------------------------------------------------------------

    #[test]
    fn test_schema_with_no_defs_root_extraction() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        let result = extract_component(&schema, "#", &opts()).unwrap();
        assert_eq!(result.dependency_count, 0);
        assert!(result.schema["properties"]["name"].is_object());
        assert!(result.schema.get("$defs").is_none());
    }

    // -----------------------------------------------------------------------
    // max_depth exceeded
    // -----------------------------------------------------------------------

    #[test]
    fn test_max_depth_exceeded() {
        let schema = json!({
            "$defs": {
                "A": { "properties": { "b": { "$ref": "#/$defs/B" } } },
                "B": { "properties": { "c": { "$ref": "#/$defs/C" } } },
                "C": { "properties": { "d": { "$ref": "#/$defs/D" } } },
                "D": { "properties": { "e": { "$ref": "#/$defs/E" } } },
                "E": { "type": "string" }
            }
        });

        let limited = ExtractOptions { max_depth: Some(1) };
        let err = extract_component(&schema, "#/$defs/A", &limited).unwrap_err();
        match err {
            ConvertError::RecursionDepthExceeded { max_depth, .. } => {
                assert_eq!(max_depth, 1);
            }
            other => panic!("expected RecursionDepthExceeded, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Key collision: two pointers with same last segment
    // -----------------------------------------------------------------------

    #[test]
    fn test_ref_rewrite_name_collision() {
        // #/components/schemas/Name and #/components/parameters/Name both referenced
        let schema = json!({
            "components": {
                "schemas": {
                    "Pet": {
                        "type": "object",
                        "properties": {
                            "a": { "$ref": "#/components/schemas/Name" },
                            "b": { "$ref": "#/components/parameters/Name" }
                        }
                    },
                    "Name": { "type": "string" }
                },
                "parameters": {
                    "Name": { "type": "integer" }
                }
            }
        });

        let result = extract_component(&schema, "#/components/schemas/Pet", &opts()).unwrap();

        // Both deps were collected
        assert_eq!(result.dependency_count, 2);
        // Keys must be distinct (no silent overwrite)
        let defs = result.schema["$defs"].as_object().unwrap();
        assert_eq!(
            defs.len(),
            2,
            "expected 2 distinct $defs keys, got: {:?}",
            defs.keys().collect::<Vec<_>>()
        );
        assert!(result.missing_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // AC: $anchor resolution in extract_component (#217)
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_resolves_anchor_ref() {
        // Schema where a property references another def via $anchor.
        let schema = json!({
            "$defs": {
                "step-object": {
                    "$anchor": "stepId",
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                },
                "workflow-object": {
                    "type": "object",
                    "properties": {
                        "step": { "$ref": "#stepId" }
                    }
                }
            }
        });
        let result = extract_component(&schema, "#/$defs/workflow-object", &opts())
            .expect("extract_component should resolve $anchor refs");
        // step-object should be pulled in as a dependency.
        assert_eq!(
            result.dependency_count, 1,
            "step-object should be a dependency"
        );
        // The $ref should be rewritten to point to the new $defs location.
        let step_ref = result.schema["properties"]["step"]["$ref"]
            .as_str()
            .expect("step.$ref should exist");
        assert!(
            step_ref.starts_with("#/$defs/"),
            "step.$ref should be rewritten to #/$defs/...; got: {}",
            step_ref
        );
    }

    #[test]
    fn test_extract_anchor_ref_with_id_scoping() {
        // Schema where $id changes the base URI and $anchor is scoped.
        let schema = json!({
            "$id": "https://example.com/root.json",
            "$defs": {
                "nested": {
                    "$id": "nested.json",
                    "$anchor": "foo",
                    "type": "string"
                },
                "consumer": {
                    "type": "object",
                    "properties": {
                        "val": { "$ref": "nested.json#foo" }
                    }
                }
            }
        });
        let result = extract_component(&schema, "#/$defs/consumer", &opts())
            .expect("extract_component should resolve $id-scoped $anchor refs");
        assert_eq!(result.dependency_count, 1);
    }

    // -----------------------------------------------------------------------
    // DependencyGraph — TDD acceptance tests (#190)
    // -----------------------------------------------------------------------

    #[test]
    fn test_dep_graph_build_basic() {
        let schema = json!({
            "$defs": {
                "A": { "type": "object", "properties": { "b": { "$ref": "#/$defs/B" } } },
                "B": { "type": "integer" },
                "C": { "type": "string" }
            }
        });
        let graph = DependencyGraph::build(&schema, &opts()).unwrap();
        // A depends on B; B and C have no deps.
        let a_result = graph.extract("#/$defs/A", &opts()).unwrap();
        assert_eq!(a_result.dependency_count, 1);
        assert!(a_result.schema["$defs"]["B"].is_object());

        let b_result = graph.extract("#/$defs/B", &opts()).unwrap();
        assert_eq!(b_result.dependency_count, 0);

        let c_result = graph.extract("#/$defs/C", &opts()).unwrap();
        assert_eq!(c_result.dependency_count, 0);
    }

    #[test]
    fn test_dep_graph_extract_matches_extract_component() {
        let schema = json!({
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "b": { "$ref": "#/$defs/B" },
                        "c": { "$ref": "#/$defs/C" }
                    }
                },
                "B": { "type": "object", "properties": { "c": { "$ref": "#/$defs/C" } } },
                "C": { "type": "string" }
            }
        });
        let graph = DependencyGraph::build(&schema, &opts()).unwrap();
        let pointers = list_components(&schema);

        for pointer in &pointers {
            let expected = extract_component(&schema, pointer, &opts()).unwrap();
            let actual = graph.extract(pointer, &opts()).unwrap();

            // Structural equality
            assert_eq!(
                actual.schema, expected.schema,
                "schema mismatch for {pointer}"
            );
            assert_eq!(
                actual.dependency_count, expected.dependency_count,
                "dep count mismatch for {pointer}"
            );
            assert_eq!(
                actual.missing_refs, expected.missing_refs,
                "missing_refs mismatch for {pointer}"
            );

            // Serialized equality (catches key order drift)
            let actual_json = serde_json::to_string(&actual.schema).unwrap();
            let expected_json = serde_json::to_string(&expected.schema).unwrap();
            assert_eq!(
                actual_json, expected_json,
                "serialized schema mismatch for {pointer}"
            );
        }
    }

    #[test]
    fn test_dep_graph_circular_refs() {
        let schema = json!({
            "$defs": {
                "A": { "type": "object", "properties": { "b": { "$ref": "#/$defs/B" } } },
                "B": { "type": "object", "properties": { "a": { "$ref": "#/$defs/A" } } }
            }
        });
        // Must not panic or infinite loop
        let graph = DependencyGraph::build(&schema, &opts()).unwrap();
        let result = graph.extract("#/$defs/A", &opts()).unwrap();
        assert!(result.dependency_count >= 1);
        assert!(result.missing_refs.is_empty());
    }

    #[test]
    fn test_dep_graph_oas_style() {
        let schema = json!({
            "components": {
                "schemas": {
                    "Pet": {
                        "type": "object",
                        "properties": {
                            "tag": { "$ref": "#/components/schemas/Tag" }
                        }
                    },
                    "Tag": { "type": "object", "properties": { "id": { "type": "integer" } } }
                }
            }
        });
        let graph = DependencyGraph::build(&schema, &opts()).unwrap();
        let result = graph.extract("#/components/schemas/Pet", &opts()).unwrap();
        assert_eq!(result.dependency_count, 1);
        assert!(result.schema["$defs"]["Tag"].is_object());
        assert_eq!(
            result.schema["properties"]["tag"]["$ref"],
            json!("#/$defs/Tag")
        );
    }

    #[test]
    fn test_dep_graph_shared_deps() {
        // A→C and B→C: C should be correctly handled for both extractions
        let schema = json!({
            "$defs": {
                "A": { "type": "object", "properties": { "c": { "$ref": "#/$defs/C" } } },
                "B": { "type": "object", "properties": { "c": { "$ref": "#/$defs/C" } } },
                "C": { "type": "string" }
            }
        });
        let graph = DependencyGraph::build(&schema, &opts()).unwrap();
        let a = graph.extract("#/$defs/A", &opts()).unwrap();
        let b = graph.extract("#/$defs/B", &opts()).unwrap();
        assert_eq!(a.dependency_count, 1);
        assert_eq!(b.dependency_count, 1);
        assert!(a.schema["$defs"]["C"].is_object());
        assert!(b.schema["$defs"]["C"].is_object());
    }
}
