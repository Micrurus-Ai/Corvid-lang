// Typed errors for a Corvid application (slice 51l).
//
// A Corvid backend describes its error enums with `@status` codes
// (slice 51e), so the client surfaces failures as a structured
// `CorvidError` carrying the HTTP status and the parsed body. Generated
// code narrows on `error.body.tag` against the discriminated-union type
// for exhaustive handling.

export class CorvidError extends Error {
  constructor(
    /** HTTP status code. */
    readonly status: number,
    /** The parsed error body, if the response was JSON. */
    readonly body: unknown,
    message?: string,
  ) {
    super(message ?? `Corvid request failed with status ${status}`);
    this.name = "CorvidError";
  }

  /** The variant tag of a typed error enum body, when present. */
  get tag(): string | undefined {
    if (this.body && typeof this.body === "object" && "tag" in this.body) {
      return String((this.body as { tag: unknown }).tag);
    }
    return undefined;
  }
}
