export async function instantiate() {
    return {
        first_row(scroll: bigint, height: bigint) {
            const first = scroll / height;
            return first < 2n ? 0n : first - 2n;
        },
        end_row(scroll: bigint, viewport: bigint, height: bigint, count: bigint) {
            const end = (scroll + viewport + height - 1n) / height + 2n;
            return end > count ? count : end;
        },
        stock(bin: bigint) {
            return bin * 17n % 101n;
        },
    };
}
