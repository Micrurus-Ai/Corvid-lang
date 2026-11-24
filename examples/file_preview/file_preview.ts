import type { BlobBinding, ClipboardBinding } from "./target/file_access.js";

class FilePreview extends HTMLElement {
    private listeners?: AbortController;
    private observer?: MutationObserver;
    private worker?: Worker;
    private selection = 0;

    start_preview() {
        const picker = this.querySelector("input")!;
        const preview = this.querySelector("pre")!;
        const copy = this.querySelector("button")!;
        const status = this.querySelector("output")!;
        this.listeners = new AbortController();
        const { signal } = this.listeners;
        status.value = preview.textContent ? "Preview ready." : "Choose a text file.";
        const syncCopy = () => { copy.disabled = !preview.textContent; };
        if (typeof MutationObserver === "function") {
            this.observer = new MutationObserver(syncCopy);
            this.observer.observe(preview, { childList: true, characterData: true, subtree: true });
        }
        syncCopy();
        picker.addEventListener("change", async () => {
            const selection = ++this.selection;
            this.stopWorker();
            preview.textContent = "";
            syncCopy();
            const file = picker.files?.[0];
            if (!file) {
                status.value = "Choose a text file.";
                return;
            }
            if (typeof Worker !== "function" || typeof file.arrayBuffer !== "function") {
                status.value = "Preview unavailable. Open the file in a text editor.";
                return;
            }
            const failed = () => {
                if (selection !== this.selection) return;
                this.stopWorker();
                status.value = "Could not preview this file. Open it in a text editor.";
            };
            try {
                status.value = "Reading file…";
                const blob: BlobBinding = file;
                const bytes = await blob.arrayBuffer();
                if (selection !== this.selection) return;
                const worker = new Worker(new URL("./decode_file.js", import.meta.url));
                this.worker = worker;
                worker.onerror = failed;
                worker.onmessageerror = failed;
                worker.onmessage = (event: MessageEvent<string>) => {
                    if (selection !== this.selection) return;
                    preview.textContent = event.data;
                    if (!this.observer) syncCopy();
                    status.value = "Preview ready.";
                    this.stopWorker();
                };
                worker.postMessage(bytes, [bytes]);
            } catch {
                failed();
            }
        }, { signal });
        copy.addEventListener("click", async () => {
            const selection = this.selection;
            try {
                const clipboard: ClipboardBinding | undefined = navigator.clipboard;
                if (typeof clipboard?.writeText !== "function") {
                    status.value = "Clipboard unavailable. Select and copy the preview.";
                    return;
                }
                await clipboard.writeText(preview.textContent ?? "");
                if (selection === this.selection) status.value = "Copied.";
            } catch {
                if (selection === this.selection) status.value = "Copy failed. Select and copy the preview.";
            }
        }, { signal });
    }

    private stopWorker() {
        this.worker?.terminate();
        this.worker = undefined;
    }

    stop_preview() {
        ++this.selection;
        this.listeners?.abort();
        this.observer?.disconnect();
        this.observer = undefined;
        this.stopWorker();
    }
}
customElements.define("file-preview", FilePreview);
