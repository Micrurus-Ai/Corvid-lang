function scoreBatch(count) {
    let total = 0;
    for (let i = 0; i < count; i++) {
        const score = 3 * (i % 257) + 5 * ((17 * i) % 257) + 7 * ((31 * i) % 257);
        if (score >= 2000) total += score;
    }
    return total;
}

console.log(scoreBatch(Number(process.argv[2])));
