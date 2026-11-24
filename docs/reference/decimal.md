# Exact decimals

`Decimal` holds a base-10 number exactly. Use it for money, prices, rates and anything a person would add up on paper and expect to balance. Arithmetic on decimals is exact or it traps, and the only operations that drop digits are the rounding methods, each of which names its mode.

```corvid
type Line:
    name: String
    price: Decimal
    quantity: Int

agent invoice(lines: List<Line>) -> String:
    subtotal: Decimal = 0
    for line in lines:
        subtotal = subtotal + line.price * line.quantity
    tax = (subtotal * 0.0825).round_half_even(2)
    share = (subtotal / 3).round_half_even(2)
    return subtotal.to_string() + " + " + tax.to_string() + " tax, " + share.to_string() + " each"
```

With lines of 4.50 × 2, 2.25 × 3 and 0.10 × 3, this returns `16.05 + 1.32 tax, 5.35 each`. The same program with `Float` would add `0.1 + 0.2` as `0.30000000000000004`.

`Decimal` runs on the interpreter. Native builds refuse programs that use it until its native slice lands, and it cannot yet cross a tool, prompt, connector, route, store or table boundary, because its JSON form arrives with B06's JSON slice. The [B06 proposal](../proposals/b06-decimal-and-time.md) has the plan.

## Writing a decimal

A number literal is a `Decimal` wherever a `Decimal` is meant, and the compiler reads it from the digits you wrote:

| You write | It is |
| --- | --- |
| `price: Decimal = 19.99` | a `Decimal`, because the binding is declared `Decimal` |
| `Line("Coffee", 4.50, 2)` | `4.50` is a `Decimal`, because the field is |
| `price * 1.08` | `1.08` is a `Decimal`, because `price` is |
| `total: Decimal = 0.1 + 0.2` | exactly `0.3` |
| `x = 0.1 + 0.2` | a `Float`: nothing here asks for a `Decimal` |

A literal keeps its scale, so `2.50` prints as `2.50`. A literal with more than 28 digits after the point, or more than 38 significant digits, is a compile error. An integer literal must fit in `Int`; write a larger whole number with a point, such as `100000000000000000000.0`.

Text that arrives at run time goes through `text.parse_decimal()`, which trims whitespace like `parse_int` and returns `Result<Decimal, String>`. It accepts an optional sign, digits, and optionally a point followed by digits. It rejects exponents (`1e3`), digit separators (`1,000`) and a bare point (`.5`, `5.`), and the error names the problem and its position.

## Arithmetic

- `+`, `-`, `*`, `%`, unary `-` and the comparisons work on two decimals, or on a decimal and an `Int`, because converting an `Int` is exact.
- A sum or difference has the larger operand scale (`1.50 + 1.2` is `2.70`). A product's scale is the sum of the operand scales (`1.5 * 1.5` is `2.25`). `%` keeps the dividend's sign, as `Int`'s does.
- Equality and ordering are numeric: `1.0 == 1.00` is `true`.
- A result that needs more than 38 significant digits traps with `Decimal overflow`. A product that needs more than 28 digits after the point traps too. It never rounds silently. Decimal arithmetic is always checked; `@wrapping` does not apply to it.
- **Mixing `Decimal` and `Float` is a compile error.** Convert one side explicitly: `x.to_decimal()` or `amount.to_float()`.

## Division and rounding

Dividing decimals rarely has an exact answer, so `/` on decimals is written together with its rounding:

```corvid
share = (total / people).round_half_even(2)
```

The quotient is computed exactly and rounded once. A `/` that is not rounded right away, including `total /= 3`, is a compile error that shows this form. Dividing by the literal zero is a compile error, and dividing by a zero value traps.

Every rounding method takes `places`, the number of digits after the point (0 to 28), and returns exactly that many:

| Method | Ties and remainders | `2.5` | `-2.5` | `2.4` |
| --- | --- | --- | --- | --- |
| `round_half_even(0)` | ties to the even neighbour (banker's rounding) | `2` | `-2` | `2` |
| `round_half_up(0)` | ties away from zero (school rounding) | `3` | `-3` | `2` |
| `round_toward_zero(0)` | drops the extra digits | `2` | `-2` | `2` |
| `round_away_from_zero(0)` | any remainder moves away from zero | `3` | `-3` | `3` |
| `round_floor(0)` | toward negative infinity | `2` | `-3` | `2` |
| `round_ceiling(0)` | toward positive infinity | `3` | `-2` | `3` |

Rounding to more places pads with zeros, so `2.5` rounded with `round_half_even(2)` is `2.50`. A literal `places` outside 0 to 28 is a compile error, and a computed one traps.

## Conversions and other methods

| Call | Result |
| --- | --- |
| `n.to_decimal()` on an `Int` | exact |
| `x.to_decimal()` on a `Float` | the shortest decimal that reads back as the same Float, the digits `to_string` shows, so `0.1.to_decimal()` is `0.1`; traps on NaN and infinity |
| `d.to_float()` | the nearest Float; the one lossy conversion |
| `d.to_int_truncated()` | toward zero; traps outside the `Int` range |
| `d.to_string()` | plain digits with the value's scale, never an exponent |
| `d.abs()` | the absolute value |
| `d.scale()` | the number of digits after the point |

All of these are pure. They run in `@deterministic` code and `fn`s, and replay never needs to record them.

## Formatting money

`amount.format_currency(currency, locale)` writes an amount the way a locale writes money, from Unicode CLDR data compiled into Corvid:

```corvid
agent receipt(total: Decimal) -> String:
    return total.format_currency("EUR", "de-DE")
```

For `1234.5` this returns `1.234,50 €`. The same amount is `$1,234.50` with `"USD", "en-US"` and `1'234,50 CHF` with `"CHF", "fr-CH"`.

- **The currency and the locale are checked.** A string where a `Currency` is meant is an ISO 4217 code, and one where a `Locale` is meant is a BCP 47 tag. The compiler checks both, so `"EURO"` or a language CLDR has no data for is a compile error. Values chosen at run time, such as a customer's settings, go through `text.parse_currency()` and `text.parse_locale()`, which return a `Result`.
- **There is no default locale.** The author names it, so output never depends on the machine it runs on.
- **Formatting never rounds.** An amount with fewer digits than the currency shows is padded (`1234.5` USD shows `.50`). One with more traps: `1.005` in USD asks you to round first. `currency.minor_digits()` gives the right number of places: `total.round_half_even(euro.minor_digits())`.

| Method | Result |
| --- | --- |
| `amount.format_currency(currency, locale)` | the formatted `String` |
| `currency.minor_digits()` | 2 for USD, 0 for JPY, 3 for KWD |
| `currency.to_string()`, `locale.to_string()` | the code and the canonical tag |
| `text.parse_currency()`, `text.parse_locale()` | a `Result` for run-time text |

## Why it works this way

- **Literals take their type from where they are used.** `price * 1.08` reads the way people write prices. A separate spelling such as `Decimal("1.08")` or `1.08d` would be a second way to write a number, and the compiler already knows a decimal is meant.
- **Division carries its rounding.** Splitting 100.00 three ways must put a cent somewhere, and that choice belongs in the source. A silent default precision, such as the 28 digits Python's `decimal` uses, would make the rounding a hidden policy.
- **No implicit rounding anywhere else.** Exact-or-trap arithmetic means a program that produces a total can be trusted to have produced that total.
