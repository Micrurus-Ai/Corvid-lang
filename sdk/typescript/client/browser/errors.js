// Typed errors for a Corvid application (slice 51l).
//
// A Corvid backend describes its error enums with `@status` codes
// (slice 51e), so the client surfaces failures as a structured
// `CorvidError` carrying the HTTP status and the parsed body. Generated
// code narrows on `error.body.tag` against the discriminated-union type
// for exhaustive handling.
export class CorvidError extends Error {
    status;
    body;
    /**
     * @param status HTTP status code.
     * @param body Parsed error body, if the response was JSON.
     */
    constructor(status, body, message) {
        super(message ?? `Corvid request failed with status ${status}`);
        this.status = status;
        this.body = body;
        this.name = "CorvidError";
    }
    /** The variant tag of a typed error enum body, when present. */
    get tag() {
        if (this.body && typeof this.body === "object" && "tag" in this.body) {
            return String(this.body.tag);
        }
        return undefined;
    }
}
