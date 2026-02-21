use url::Url;

fn main() {
    let default_base = Url::parse("file:///schema.json").unwrap();
    let id_val = "folder/schema.json";
    let base_uri = default_base.join(id_val).unwrap();
    println!("Base URI: {}", base_uri);
    let scoped_base = base_uri.join(id_val).unwrap();
    println!("Scoped base: {}", scoped_base);
}
