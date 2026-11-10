#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut c = vesper_core::Check::default();
    c.url = Some(a[1].clone());
    let n = vesper_core::NetworkPolicy { allowed_cidrs: vec![a[2].clone()], ..Default::default() };
    let r = vesper_probe::run(&c, &Default::default(), &n, std::path::Path::new("/tmp")).await;
    println!("{r:?}");
}
