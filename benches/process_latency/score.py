import sys


def score_batch(count):
    total = 0
    for i in range(count):
        score = 3 * (i % 257) + 5 * ((17 * i) % 257) + 7 * ((31 * i) % 257)
        if score >= 2000:
            total += score
    return total


print(score_batch(int(sys.argv[1])))
