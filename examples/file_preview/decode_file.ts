/// <reference lib="webworker" />
const decoder = self as unknown as DedicatedWorkerGlobalScope;
decoder.onmessage = (event: MessageEvent<ArrayBuffer>) => {
    decoder.postMessage(new TextDecoder().decode(event.data));
};
