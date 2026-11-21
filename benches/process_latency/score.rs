fn main() {
    let count: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut total = 0;
    for i in 0..count {
        let score = 3 * (i % 257) + 5 * ((17 * i) % 257) + 7 * ((31 * i) % 257);
        if score >= 2000 {
            total += score;
        }
    }
    println!("{total}");
}
