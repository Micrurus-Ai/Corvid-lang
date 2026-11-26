/** Ordered state for one subscription. Key fields come from its table schema. */
export class LiveRows {
    fields;
    versions = new Map();
    order = [];
    constructor(fields) {
        if (!fields.length)
            throw new Error("Live rows require the declared table key");
        this.fields = [...fields];
    }
    key(entry) {
        if (typeof entry.revision !== "string" || !/^(0|[1-9]\d*)$/.test(entry.revision)) {
            throw new Error("Expected a decimal live revision");
        }
        return JSON.stringify(this.fields.map(field => {
            const value = entry.row[field];
            if (!Object.hasOwn(entry.row, field) ||
                !(typeof value === "string" || Number.isSafeInteger(value))) {
                throw new Error("Missing or inexact live row key");
            }
            return value;
        }));
    }
    apply(delivery) {
        if (delivery.type === "snapshot") {
            if (!Array.isArray(delivery.rows))
                throw new Error("Expected live snapshot rows");
            const next = new Map();
            for (const entry of delivery.rows) {
                const key = this.key(entry);
                if (next.has(key))
                    throw new Error("Duplicate live snapshot key");
                const old = this.versions.get(key);
                next.set(key, old?.revision === entry.revision ? old : entry);
            }
            this.versions = next;
            this.order = [...next.keys()];
        }
        else {
            const key = this.key(delivery);
            const old = this.versions.get(key);
            if (delivery.kind === "added" ? old !== undefined : old === undefined) {
                throw new Error("Live change disagrees with snapshot membership");
            }
            if (delivery.kind !== "removed" &&
                (!Number.isInteger(delivery.index) || delivery.index < 0 ||
                    delivery.index >= this.order.length + (old ? 0 : 1))) {
                throw new Error("Invalid live change index");
            }
            if (old)
                this.order.splice(this.order.indexOf(key), 1);
            if (delivery.kind === "removed") {
                this.versions.delete(key);
            }
            else {
                this.order.splice(delivery.index, 0, key);
                this.versions.set(key, old?.revision === delivery.revision ? old : delivery);
            }
        }
        return this.order.map(key => this.versions.get(key).row);
    }
}
