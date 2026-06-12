/* veil.h — the Veil OS WebAssembly app ABI (C bindings).
 *
 * A Veil app is a wasm32 module that exports `render()` (and optionally
 * `init()` and `on_click(x, y)`) and draws through the host functions below.
 * The host dispatches imports by name, so the import module ("env") is just the
 * wasm default.
 *
 * Build with a wasm-capable clang (e.g. the WASI SDK or upstream LLVM):
 *
 *   clang --target=wasm32 -nostdlib -O2 \
 *     -Wl,--no-entry -Wl,--export=render -Wl,--export=on_click \
 *     -Wl,--export=init -Wl,--allow-undefined \
 *     -o hello.wasm hello.c
 *   cp hello.wasm HELLO.WSM     # then open it in Veil's file manager
 */
#ifndef VEIL_H
#define VEIL_H

#include <stdint.h>

#define VEIL_IMPORT(name) \
  __attribute__((import_module("env"), import_name(#name)))

/* ---- graphics (draw into the app window) ------------------------------- */
VEIL_IMPORT(veil_width)  int  veil_width(void);
VEIL_IMPORT(veil_height) int  veil_height(void);
VEIL_IMPORT(veil_clear)  void veil_clear(uint32_t color);
VEIL_IMPORT(veil_fill_rect)
  void veil_fill_rect(int x, int y, int w, int h, uint32_t color);
VEIL_IMPORT(veil_draw_text)
  void veil_draw_text(int x, int y, const char *ptr, int len, uint32_t color, int size);

/* ---- log / storage ----------------------------------------------------- */
VEIL_IMPORT(veil_log)       void veil_log(const char *ptr, int len);
VEIL_IMPORT(veil_store_set)
  void veil_store_set(const char *kp, int kl, const char *vp, int vl);
VEIL_IMPORT(veil_store_get)
  int  veil_store_get(const char *kp, int kl, char *out, int cap);

/* ---- network (over the kernel HTTP/TCP/TLS stack) ---------------------- */
VEIL_IMPORT(veil_http_get)
  int  veil_http_get(const char *url, int url_len, char *out, int cap);
VEIL_IMPORT(veil_http_post)
  int  veil_http_post(const char *url, int url_len, const char *body, int body_len, char *out, int cap);
VEIL_IMPORT(veil_dns_resolve) int veil_dns_resolve(const char *host, int host_len);
VEIL_IMPORT(veil_tcp_connect) int veil_tcp_connect(const char *host, int host_len, int port);
VEIL_IMPORT(veil_tcp_send)    int veil_tcp_send(int sock, const char *ptr, int len);
VEIL_IMPORT(veil_tcp_recv)    int veil_tcp_recv(int sock, char *ptr, int cap);
VEIL_IMPORT(veil_tcp_close)   void veil_tcp_close(int sock);

/* ---- app callbacks you export ------------------------------------------
 *   void render(void);             // draw the UI (called on open + each event)
 *   void init(void);               // optional: run once on open
 *   void on_click(int x, int y);   // optional: handle a click, then render()
 * State lives in your module's memory, which Veil preserves across frames.
 */

/* Convenience colors (the alpha byte is ignored by the host). */
#define VEIL_BG     0xff141414u
#define VEIL_TEXT   0xffe8e8e8u
#define VEIL_ACCENT 0xff5b8af0u
#define VEIL_GREEN  0xff2f9e6bu
#define VEIL_GOLD   0xffffd060u
#define VEIL_WHITE  0xffffffffu

#endif /* VEIL_H */
