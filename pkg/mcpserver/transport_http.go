package mcpserver

import (
	"encoding/json"
	"io"
	"net/http"
	"sync"

	"github.com/google/uuid"
)

// newHTTPHandler returns an http.Handler that serves MCP over Streamable HTTP.
func newHTTPHandler(server *Server) http.Handler {
	h := &httpHandler{
		server:   server,
		sessions: make(map[string]bool),
	}

	mux := http.NewServeMux()
	mux.HandleFunc("POST /mcp", h.handleMCP)
	mux.HandleFunc("GET /healthz", h.handleHealth)
	return mux
}

type httpHandler struct {
	server   *Server
	mu       sync.RWMutex
	sessions map[string]bool
}

func (h *httpHandler) handleMCP(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		writeError(w, http.StatusBadRequest, "failed to read request body")
		return
	}
	defer r.Body.Close()

	var rpcReq JSONRPCRequest
	if err := json.Unmarshal(body, &rpcReq); err != nil {
		resp := NewErrorResponse(nil, ErrCodeParse, "invalid JSON: "+err.Error())
		writeJSON(w, http.StatusBadRequest, resp)
		return
	}

	// Session validation: after initialization, require Mcp-Session-Id.
	sessionID := r.Header.Get("Mcp-Session-Id")
	if rpcReq.Method != "initialize" {
		if sessionID != "" {
			h.mu.RLock()
			valid := h.sessions[sessionID]
			h.mu.RUnlock()
			if !valid {
				writeError(w, http.StatusNotFound, "session not found")
				return
			}
		}
	}

	resp := h.server.Handle(r.Context(), rpcReq)

	// Notifications: no response body.
	if resp.IsNotification() {
		w.WriteHeader(http.StatusAccepted)
		return
	}

	// On initialize, create and return a session ID.
	if rpcReq.Method == "initialize" && resp.Error == nil {
		sid := uuid.New().String()
		h.mu.Lock()
		h.sessions[sid] = true
		h.mu.Unlock()
		w.Header().Set("Mcp-Session-Id", sid)
	} else if sessionID != "" {
		w.Header().Set("Mcp-Session-Id", sessionID)
	}

	writeJSON(w, http.StatusOK, resp)
}

func (h *httpHandler) handleHealth(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	w.Write([]byte(`{"status":"ok"}`))
}

func writeJSON(w http.ResponseWriter, status int, payload any) {
	buf, err := json.Marshal(payload)
	if err != nil {
		w.WriteHeader(http.StatusInternalServerError)
		w.Write([]byte(`{"error":"marshal failure"}`))
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	w.Write(buf)
}

func writeError(w http.ResponseWriter, status int, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(map[string]string{"error": msg})
}
