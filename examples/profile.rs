//! Small driver for profilers.
//! `profile c <level> <file> [mib]` compresses; `profile d <file.deflate>` decompresses 3 times.
use verified_deflate::{compress, decompress};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "c" {
        let mut data = std::fs::read(&a[3]).unwrap();
        let mb: usize = a.get(4).map(|x| x.parse().unwrap()).unwrap_or(1);
        data.truncate(mb << 20);
        let c = compress(&data, a[2].parse().unwrap());
        println!("{}", c.len());
    } else {
        let c = std::fs::read(&a[2]).unwrap();
        let mut n = 0;
        for _ in 0..3 {
            n += decompress(&c).unwrap().len();
        }
        println!("{n}");
    }
}
