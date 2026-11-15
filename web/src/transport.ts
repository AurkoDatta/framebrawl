// The two ways to reach the server. WebRTC gives an unreliable channel for
// per-tick traffic and a reliable one for everything else. The WebSocket
// carries both kinds on one ordered stream, so it works anywhere but a
// lost packet delays everything behind it.

export type TransportKind = "webrtc" | "websocket";

export interface Transport {
  readonly kind: TransportKind;
  send(bytes: Uint8Array, reliable: boolean): void;
  close(): void;
  onmessage: (bytes: Uint8Array) => void;
  onclose: () => void;
}

export interface ConnectOptions {
  room: string | null;
  /** Skip WebRTC and go straight to the WebSocket. */
  websocketOnly: boolean;
  /** How long WebRTC gets before the WebSocket is used instead. */
  webrtcTimeoutMs: number;
}

function query(room: string | null): string {
  return room ? `?room=${encodeURIComponent(room)}` : "";
}

export async function connect(options: ConnectOptions): Promise<Transport> {
  if (!options.websocketOnly && "RTCPeerConnection" in globalThis) {
    try {
      return await connectWebRtc(options.room, options.webrtcTimeoutMs);
    } catch (error) {
      console.info("WebRTC unavailable, using WebSocket:", error);
    }
  }
  return connectWebSocket(options.room);
}

async function connectWebRtc(room: string | null, timeoutMs: number): Promise<Transport> {
  const peer = new RTCPeerConnection();
  const frames = peer.createDataChannel("frames", { ordered: false, maxRetransmits: 0 });
  const control = peer.createDataChannel("control");
  frames.binaryType = "arraybuffer";
  control.binaryType = "arraybuffer";

  const transport: Transport = {
    kind: "webrtc",
    send(bytes, reliable) {
      const channel = reliable ? control : frames;
      if (channel.readyState === "open") {
        channel.send(bytes as Uint8Array<ArrayBuffer>);
      }
    },
    close() {
      peer.close();
    },
    onmessage: () => {},
    onclose: () => {},
  };

  // Messages can arrive the moment a channel opens, before the caller has
  // attached its handler. Hold them until it does.
  const early: Uint8Array[] = [];
  let live = false;
  const deliver = (event: MessageEvent<ArrayBuffer>) => {
    const bytes = new Uint8Array(event.data);
    if (live) {
      transport.onmessage(bytes);
    } else {
      early.push(bytes);
    }
  };
  frames.onmessage = deliver;
  control.onmessage = deliver;

  const opened = new Promise<void>((resolve, reject) => {
    const check = () => {
      if (frames.readyState === "open" && control.readyState === "open") {
        resolve();
      }
    };
    frames.onopen = check;
    control.onopen = check;
    peer.onconnectionstatechange = () => {
      if (peer.connectionState === "failed") {
        reject(new Error("connection failed"));
      }
    };
    setTimeout(() => reject(new Error(`no connection after ${timeoutMs} ms`)), timeoutMs);
  });

  try {
    // The server has fixed addresses and learns ours from the first probe
    // we send, so the offer can go out without waiting for candidates.
    await peer.setLocalDescription(await peer.createOffer());
    const response = await fetch(`/api/rtc${query(room)}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(peer.localDescription),
    });
    if (!response.ok) {
      throw new Error(`signalling returned ${response.status}`);
    }
    await peer.setRemoteDescription(await response.json());
    await opened;
  } catch (error) {
    peer.close();
    throw error;
  }

  let closed = false;
  const close = () => {
    if (!closed) {
      closed = true;
      transport.onclose();
    }
  };
  control.onclose = close;
  peer.onconnectionstatechange = () => {
    if (peer.connectionState === "failed" || peer.connectionState === "closed") {
      close();
    }
  };

  queueMicrotask(() => {
    live = true;
    for (const bytes of early) {
      transport.onmessage(bytes);
    }
    early.length = 0;
  });
  return transport;
}

function connectWebSocket(room: string | null): Promise<Transport> {
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(`${scheme}//${location.host}/api/ws${query(room)}`);
  socket.binaryType = "arraybuffer";

  const transport: Transport = {
    kind: "websocket",
    send(bytes) {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(bytes as Uint8Array<ArrayBuffer>);
      }
    },
    close() {
      socket.close();
    },
    onmessage: () => {},
    onclose: () => {},
  };

  return new Promise((resolve, reject) => {
    socket.onopen = () => resolve(transport);
    socket.onerror = () => reject(new Error("WebSocket failed to connect"));
    socket.onmessage = (event: MessageEvent<ArrayBuffer>) => {
      transport.onmessage(new Uint8Array(event.data));
    };
    socket.onclose = () => transport.onclose();
  });
}
