const start = document.querySelector("#start");
const stop = document.querySelector("#stop");
const video = document.querySelector("video");
const status = document.querySelector("#camera-status");
let request = 0;

function closeCamera(message) {
  request++;
  video.srcObject?.getTracks().forEach(track => track.stop());
  video.srcObject = null;
  video.hidden = true;
  start.disabled = !navigator.mediaDevices?.getUserMedia;
  stop.disabled = true;
  status.textContent = message;
}

start.onclick = async () => {
  const current = ++request;
  start.disabled = true;
  stop.disabled = false;
  status.textContent = "Requesting camera.";
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
    if (current !== request) {
      stream.getTracks().forEach(track => track.stop());
      return;
    }
    stream.getVideoTracks()[0].addEventListener("ended", () => {
      if (current === request) closeCamera("Camera disconnected.");
    });
    video.srcObject = stream;
    video.hidden = false;
    await video.play();
    if (current === request) status.textContent = "Camera live.";
  } catch (error) {
    if (current === request) closeCamera(error.name === "NotAllowedError"
      ? "Camera permission denied. You can continue editing." : "Camera unavailable.");
  }
};
stop.onclick = () => closeCamera("Camera off.");
window.addEventListener("pagehide", () => closeCamera("Camera off."));
if (!navigator.mediaDevices?.getUserMedia) closeCamera("Camera unavailable.");
