// Building a declared route's URL from typed values.
/**
 * Fill the `{name}` segments of a contract path. Each value is
 * percent-encoded as one segment, and a number that is not a safe integer
 * is refused rather than rounded, because the server would read a
 * different `Int` than the caller wrote.
 */
export function fillPath(template, values) {
    return template.replace(/\{([^{}]+)\}/g, (_, name) => {
        if (!Object.hasOwn(values, name))
            throw new Error("Missing path argument " + name);
        const value = values[name];
        if (typeof value === "number" && !Number.isSafeInteger(value)) {
            throw new Error("Inexact path argument " + name);
        }
        if (typeof value !== "string" && typeof value !== "number") {
            throw new Error("Path argument " + name + " must be a string or an integer");
        }
        return encodeURIComponent(String(value));
    });
}
/**
 * A record as `name=value` pairs, the form the server reads a route's
 * query string and a form body in: a missing or `null` field is left
 * out, and a list repeats its name.
 */
export function urlEncodedRecord(record) {
    const params = new URLSearchParams();
    if (record && typeof record === "object") {
        for (const [name, value] of Object.entries(record)) {
            if (value === null || value === undefined)
                continue;
            for (const item of Array.isArray(value) ? value : [value])
                params.append(name, String(item));
        }
    }
    return params.toString();
}
