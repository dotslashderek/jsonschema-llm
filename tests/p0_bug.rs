use jsonschema_llm_core::passes::p0_normalize::normalize;
use jsonschema_llm_core::config::ConvertOptions;
use serde_json::json;

fn main() {
    let schema = json!({
        "$id": "subfolder/schema.json",
        "$defs": {
            "foo": {
                "$id": "nested/foo.json",
                "$anchor": "my-anchor",
                "type": "string"
            }
        },
        "properties": {
            "a": { "$ref": "#my-anchor" }
        }
    });
    
    let config = ConvertOptions::default();
    match normalize(&schema, &config) {
        Ok(res) => println!("Success: {}", res.pass.schema),
        Err(e) => println!("Error: {}", e),
    }
}
