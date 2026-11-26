/** Fill the named segments of a normalized contract path in one pass. */
export function livePath(template, values) {
    return template.replace(/\{([^{}]+)\}/g, (_, name) => {
        if (!Object.hasOwn(values, name))
            throw new Error("Missing live path argument " + name);
        const value = values[name];
        if (typeof value === "number" && !Number.isSafeInteger(value))
            throw new Error("Inexact live path argument " + name);
        return encodeURIComponent(String(value));
    });
}
