// Operations used from https://w3c.github.io/FileAPI/#blob-section
partial interface Blob {
    Promise<ArrayBuffer> arrayBuffer();
};

// Operation used from https://w3c.github.io/clipboard-apis/#clipboard-interface
partial interface Clipboard {
    Promise<undefined> writeText(DOMString data);
};
