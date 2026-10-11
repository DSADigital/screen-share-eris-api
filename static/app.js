/*
 * Éris Screen Share — cliente de demonstração.
 *
 * Fluxo:
 *   1. POST /api/demo/identity          → identidade descartável (token em memória da aba)
 *   2. POST /api/screen-share/sessions   → cria sessão (ou /sessions/{code}/join)
 *   3. WebSocket /api/screen-share/ws    → sinalização (controle + SDP/ICE)
 *   4. RTCPeerConnection                 → vídeo P2P direto entre navegadores
 *
 * Nada é salvo em cookies, localStorage ou enviado a terceiros.
 */
(() => {
  "use strict";

  // Servidores ICE. Vazio por padrão = nenhuma requisição a terceiros; funciona
  // na mesma máquina/rede local. Para redes diferentes, informe um STUN/TURN
  // seu via ?stun=stun:host:porta (opt-in explícito).
  const params = new URLSearchParams(location.search);
  const ICE_SERVERS = params.getAll("stun").map((urls) => ({ urls }));

  const $ = (id) => document.getElementById(id);
  const el = {
    lobby: $("view-lobby"), session: $("view-session"), ended: $("view-ended"),
    name: $("input-name"), code: $("input-code"),
    create: $("btn-create"), join: $("btn-join"),
    sessionCode: $("session-code"), copy: $("btn-copy"), status: $("session-status"),
    video: $("video"), placeholder: $("video-placeholder"), placeholderText: $("placeholder-text"), videoLabel: $("video-label"),
    share: $("btn-share"), swap: $("btn-swap"), stop: $("btn-stop"), leave: $("btn-leave"), close: $("btn-close"),
    participants: $("participants"), peerState: $("peer-state"),
    log: $("log"), clearLog: $("btn-clear-log"),
    endedReason: $("ended-reason"), back: $("btn-back"),
    modal: $("swap-modal"), swapName: $("swap-name"), swapAccept: $("btn-swap-accept"), swapRefuse: $("btn-swap-refuse"),
    toasts: $("toasts"),
  };

  const state = {
    token: null, me: null,
    sessionId: null, session: null, broadcasterId: null,
    ws: null, queue: Promise.resolve(),
    pc: null, pendingCandidates: [],
    localStream: null,   // stream sendo transmitido
    pendingStream: null, // stream capturado aguardando confirmação do servidor
  };

  // ------------------------------------------------------------------ UI utils
  function toast(msg, kind = "") {
    if (!msg) return;
    const t = document.createElement("div");
    t.className = `toast ${kind}`;
    t.textContent = msg;
    el.toasts.appendChild(t);
    setTimeout(() => t.remove(), 4200);
  }

  function show(view) {
    for (const v of [el.lobby, el.session, el.ended]) v.classList.toggle("hidden", v !== view);
  }

  /** Resume payloads grandes (SDP) para manter o log legível. */
  function summarize(obj) {
    return JSON.stringify(obj, (k, v) =>
      (k === "sdp" || k === "ticket" || k === "token") && typeof v === "string" && v.length > 48
        ? `${v.slice(0, 48)}… (${v.length} chars)` : v);
  }

  function log(kind, label, body) {
    const li = document.createElement("li");
    li.className = kind;
    const dir = { out: "↑", in: "↓", http: "⇄" }[kind];
    li.innerHTML = `<span class="dir">${dir}</span><span class="ev"></span><span class="body"></span>`;
    li.querySelector(".ev").textContent = label;
    li.querySelector(".body").textContent = body === undefined ? "" : summarize(body);
    el.log.appendChild(li);
    el.log.scrollTop = el.log.scrollHeight;
  }

  // ------------------------------------------------------------------ HTTP
  async function api(method, path, body) {
    const headers = { "Content-Type": "application/json" };
    if (state.token) headers.Authorization = `Bearer ${state.token}`;
    log("http", `${method} ${path}`, body);
    const res = await fetch(path, { method, headers, body: body ? JSON.stringify(body) : undefined });
    const json = await res.json().catch(() => ({}));
    log("http", `${res.status} ${json.status ?? ""}`, json.data && Object.keys(json.data).length ? json.data : undefined);
    if (!res.ok || json.status !== "ok") throw new Error(json.toast || json.message || `Erro HTTP ${res.status}`);
    return json.data;
  }

  async function ensureIdentity() {
    const first_name = el.name.value.trim();
    if (!first_name) { el.name.focus(); throw new Error("Informe seu nome de exibição."); }
    if (state.token && state.me.first_name === first_name) return;
    const data = await api("POST", "/api/demo/identity", { first_name });
    state.token = data.token;
    state.me = { id: data.user_id, username: data.username, first_name: data.first_name };
  }

  // ------------------------------------------------------------------ WebSocket
  function connect(url, ticket, sessionId) {
    state.sessionId = sessionId;
    const ws = new WebSocket(`${url}?ticket=${encodeURIComponent(ticket)}`);
    state.ws = ws;
    ws.onmessage = (e) => {
      let msg;
      try { msg = JSON.parse(e.data); } catch { return; }
      log("in", msg.event, msg.payload);
      // Processa mensagens em ordem estrita (SDP antes de ICE, etc.)
      state.queue = state.queue.then(() => handle(msg)).catch((err) => console.error(err));
    };
    ws.onclose = () => {
      if (state.ws === ws && state.sessionId) {
        log("in", "websocket fechado");
        setPeerState("—");
      }
    };
    show(el.session);
  }

  function send(action, payload) {
    if (!state.ws || state.ws.readyState !== WebSocket.OPEN) return;
    const msg = payload === undefined ? { action } : { action, payload };
    log("out", action, payload);
    state.ws.send(JSON.stringify(msg));
  }

  // ------------------------------------------------------------------ Eventos do servidor
  async function handle({ event, payload }) {
    switch (event) {
      case "session_state":
        state.session = payload;
        state.broadcasterId = payload.broadcaster_id;
        break;

      case "participant_joined":
        toast(`${payload.first_name} entrou na sessão.`, "ok");
        // Se eu já transmito, renegocia para o novo par receber o vídeo.
        if (amBroadcaster() && state.localStream) await startAsBroadcaster();
        break;

      case "participant_left":
        toast("O outro participante saiu ou desconectou.");
        teardownPeer();
        await refreshState();
        break;

      case "broadcast_started":
        state.broadcasterId = payload.broadcaster_id;
        if (payload.broadcaster_id === state.me.id) {
          if (!state.pendingStream) { send("stop_broadcast"); break; }
          state.localStream = state.pendingStream;
          state.pendingStream = null;
          await startAsBroadcaster();
        } else {
          releasePending();
        }
        await refreshState();
        break;

      case "broadcast_stopped":
        state.broadcasterId = null;
        teardownPeer();
        stopStream("localStream");
        await refreshState();
        break;

      case "swap_requested":
        el.swapName.textContent = payload.requester_name || "O outro participante";
        el.modal.classList.remove("hidden");
        break;

      case "swap_response":
        if (!payload.accepted) { releasePending(); toast("O pedido de troca foi recusado.", "err"); }
        else toast("Troca de transmissão realizada.", "ok");
        break;

      case "sdp_offer": await onOffer(payload.sdp); break;
      case "sdp_answer":
        if (state.pc) {
          await state.pc.setRemoteDescription({ type: "answer", sdp: payload.sdp });
          await flushCandidates();
        }
        break;
      case "ice_candidate": await onRemoteCandidate(payload); break;

      case "session_closed":
        endSession(payload.reason);
        return;

      case "error":
        toast(payload.message, "err");
        if (["start_broadcast_failed", "request_swap_failed"].includes(payload.code)) releasePending();
        break;
    }
    render();
  }

  async function refreshState() {
    if (!state.sessionId) return;
    try {
      state.session = await api("GET", `/api/screen-share/sessions/${state.sessionId}`);
      state.broadcasterId = state.session.broadcaster_id;
    } catch { /* sessão pode ter sido encerrada */ }
  }

  // ------------------------------------------------------------------ WebRTC
  const amBroadcaster = () => !!state.me && state.broadcasterId === state.me.id;

  function createPeer() {
    teardownPeer();
    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS });
    state.pc = pc;
    state.pendingCandidates = [];
    pc.onicecandidate = ({ candidate }) => {
      if (!candidate) return;
      send("ice_candidate", {
        candidate: candidate.candidate,
        sdp_mid: candidate.sdpMid,
        sdp_m_line_index: candidate.sdpMLineIndex,
      });
    };
    pc.ontrack = ({ streams }) => {
      el.video.srcObject = streams[0];
      el.video.play().catch(() => {});
      render();
    };
    pc.onconnectionstatechange = () => setPeerState(pc.connectionState);
    setPeerState("negociando");
    return pc;
  }

  async function startAsBroadcaster() {
    const pc = createPeer();
    for (const track of state.localStream.getTracks()) pc.addTrack(track, state.localStream);
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    send("sdp_offer", { sdp: offer.sdp });
  }

  async function onOffer(sdp) {
    const pc = createPeer();
    await pc.setRemoteDescription({ type: "offer", sdp });
    await flushCandidates();
    const answer = await pc.createAnswer();
    await pc.setLocalDescription(answer);
    send("sdp_answer", { sdp: answer.sdp });
  }

  async function onRemoteCandidate(p) {
    const c = { candidate: p.candidate, sdpMid: p.sdp_mid, sdpMLineIndex: p.sdp_m_line_index };
    if (!state.pc || !state.pc.remoteDescription) { state.pendingCandidates.push(c); return; }
    try { await state.pc.addIceCandidate(c); } catch (e) { console.warn(e); }
  }

  async function flushCandidates() {
    const list = state.pendingCandidates.splice(0);
    for (const c of list) { try { await state.pc.addIceCandidate(c); } catch (e) { console.warn(e); } }
  }

  function teardownPeer() {
    if (state.pc) { state.pc.ontrack = state.pc.onicecandidate = state.pc.onconnectionstatechange = null; state.pc.close(); }
    state.pc = null;
    state.pendingCandidates = [];
    el.video.srcObject = null;
    setPeerState("—");
  }

  function stopStream(key) {
    if (state[key]) state[key].getTracks().forEach((t) => t.stop());
    state[key] = null;
  }
  const releasePending = () => stopStream("pendingStream");

  function setPeerState(s) {
    el.peerState.textContent = s;
    el.peerState.className = "pill " + (s === "connected" ? "pill-ok" : s === "—" ? "pill-dim" : s === "failed" ? "pill-warn" : "");
  }

  /** Captura a tela no clique (exigência de gesto do usuário) e só então pede ao servidor. */
  async function captureThen(action) {
    let stream;
    try {
      stream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
    } catch { toast("Captura de tela cancelada."); return; }
    releasePending();
    state.pendingStream = stream;
    // Botão nativo "Parar compartilhamento" do navegador
    stream.getVideoTracks()[0].addEventListener("ended", () => {
      if (state.localStream === stream) send("stop_broadcast");
      else if (state.pendingStream === stream) releasePending();
    });
    send(action);
  }

  // ------------------------------------------------------------------ Render
  function render() {
    const s = state.session;
    if (!s) return;
    el.sessionCode.textContent = s.code;
    el.status.textContent = s.status === "waiting_for_peer" ? "aguardando par" : s.status === "ready" ? "pronta" : s.status;
    el.status.className = "pill " + (s.status === "ready" ? "pill-ok" : s.status === "closed" ? "pill-warn" : "");

    // Participantes
    el.participants.replaceChildren();
    for (const p of s.participants) {
      const li = document.createElement("li");
      const isMe = p.user_id === state.me.id;
      const live = p.user_id === state.broadcasterId;
      li.innerHTML = `<div class="avatar"></div><div class="p-name"><span class="n"></span><small></small></div>${live ? '<span class="tag-live">AO VIVO</span>' : ""}<span class="dot ${p.is_connected ? "on" : ""}"></span>`;
      li.querySelector(".avatar").textContent = (p.first_name || "?").charAt(0).toUpperCase();
      li.querySelector(".n").textContent = p.first_name + (isMe ? " (você)" : "");
      li.querySelector("small").textContent = p.username;
      el.participants.appendChild(li);
    }
    if (s.participants.length < 2) {
      const li = document.createElement("li");
      li.className = "empty-slot";
      li.textContent = "Aguardando o segundo participante…";
      el.participants.appendChild(li);
    }

    // Controles
    const someoneElse = state.broadcasterId && !amBroadcaster();
    el.share.classList.toggle("hidden", !!state.broadcasterId);
    el.swap.classList.toggle("hidden", !someoneElse);
    el.stop.classList.toggle("hidden", !amBroadcaster());
    el.swap.disabled = s.is_swapping;
    el.swap.textContent = s.is_swapping ? "Troca solicitada…" : "Solicitar troca";

    // Vídeo
    const broadcaster = s.participants.find((p) => p.user_id === state.broadcasterId);
    if (amBroadcaster() && state.localStream) {
      if (el.video.srcObject !== state.localStream) el.video.srcObject = state.localStream;
    }
    const hasVideo = !!el.video.srcObject;
    el.placeholder.classList.toggle("hidden", hasVideo);
    el.videoLabel.classList.toggle("hidden", !hasVideo);
    el.videoLabel.textContent = amBroadcaster() ? "Você está transmitindo" : broadcaster ? `${broadcaster.first_name} está transmitindo` : "";
    el.placeholderText.textContent = broadcaster
      ? `Conectando ao vídeo de ${broadcaster.first_name}…`
      : s.participants.length < 2 ? `Envie o código ${s.code} para o outro participante.` : "Ninguém está transmitindo ainda.";
  }

  // ------------------------------------------------------------------ Ciclo de vida
  function resetLocal() {
    teardownPeer();
    stopStream("localStream");
    releasePending();
    const ws = state.ws;
    Object.assign(state, { sessionId: null, session: null, broadcasterId: null, ws: null });
    if (ws) ws.close();
    el.modal.classList.add("hidden");
  }

  function endSession(reason) {
    resetLocal();
    el.endedReason.textContent = reason || "Recursos liberados.";
    show(el.ended);
  }

  async function guarded(btn, fn) {
    btn.disabled = true;
    try { await fn(); } catch (e) { toast(e.message, "err"); } finally { btn.disabled = false; }
  }

  el.create.addEventListener("click", () => guarded(el.create, async () => {
    await ensureIdentity();
    const d = await api("POST", "/api/screen-share/sessions");
    connect(d.websocket_url, d.ticket, d.session_id);
    toast("Sessão criada! Compartilhe o código.", "ok");
  }));

  el.join.addEventListener("click", () => guarded(el.join, async () => {
    const code = el.code.value.trim().toUpperCase();
    if (!code) { el.code.focus(); throw new Error("Informe o código da sessão."); }
    await ensureIdentity();
    const d = await api("POST", `/api/screen-share/sessions/${encodeURIComponent(code)}/join`, { code });
    connect(d.websocket_url, d.ticket, d.session_id);
  }));
  el.code.addEventListener("keydown", (e) => { if (e.key === "Enter") el.join.click(); });

  el.share.addEventListener("click", () => captureThen("start_broadcast"));
  el.swap.addEventListener("click", () => captureThen("request_swap"));
  el.stop.addEventListener("click", () => send("stop_broadcast"));

  el.swapAccept.addEventListener("click", () => { el.modal.classList.add("hidden"); send("respond_swap", { accept: true }); });
  el.swapRefuse.addEventListener("click", () => { el.modal.classList.add("hidden"); send("respond_swap", { accept: false }); });

  el.leave.addEventListener("click", () => guarded(el.leave, async () => {
    const id = state.sessionId;
    resetLocal();
    await api("POST", `/api/screen-share/sessions/${id}/leave`).catch(() => {});
    show(el.lobby);
  }));

  el.close.addEventListener("click", () => guarded(el.close, async () => {
    await api("DELETE", `/api/screen-share/sessions/${state.sessionId}`);
    // O servidor também envia `session_closed` via WebSocket.
    if (state.sessionId) endSession("Sessão encerrada por você.");
  }));

  el.copy.addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(el.sessionCode.textContent); toast("Código copiado!", "ok"); }
    catch { toast("Não foi possível copiar.", "err"); }
  });

  el.clearLog.addEventListener("click", () => el.log.replaceChildren());
  el.back.addEventListener("click", () => show(el.lobby));

  // Mantém o WebSocket vivo atrás de proxies (o servidor também faz ping a cada 20s)
  setInterval(() => send("ping"), 25000);

  // Pré-preenche o código via ?code=XXXX para facilitar o convite
  const preset = params.get("code");
  if (preset) el.code.value = preset.toUpperCase();
})();
