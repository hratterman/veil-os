/* "Hello, Veil" in C — draws a title, a button, and a click counter.
 *
 *   clang --target=wasm32 -nostdlib -O2 \
 *     -Wl,--no-entry -Wl,--export=render -Wl,--export=on_click \
 *     -Wl,--export=init -Wl,--allow-undefined \
 *     -I../.. -o hello.wasm hello.c
 *   cp hello.wasm HELLOC.WSM   # open it in Veil's file manager
 */
#include "veil.h"

#define BX 20
#define BY 104
#define BW 150
#define BH 44

static int clicks = 0;

/* Tiny itoa (no libc on wasm32 -nostdlib). */
static int int_to_str(int n, char *buf) {
  if (n == 0) { buf[0] = '0'; return 1; }
  char tmp[12];
  int i = 0, neg = n < 0;
  unsigned m = neg ? (unsigned)(-(long)n) : (unsigned)n;
  while (m > 0) { tmp[i++] = '0' + (m % 10); m /= 10; }
  int j = 0;
  if (neg) buf[j++] = '-';
  while (i > 0) buf[j++] = tmp[--i];
  return j;
}

__attribute__((export_name("init")))
void init(void) { clicks = 0; }

__attribute__((export_name("render")))
void render(void) {
  veil_clear(VEIL_BG);
  const char *t1 = "Hello, Veil!";
  veil_draw_text(20, 14, t1, 12, VEIL_ACCENT, 28);
  const char *t2 = "A WebAssembly app built with the Veil SDK.";
  veil_draw_text(20, 58, t2, 42, VEIL_TEXT, 15);

  veil_fill_rect(BX, BY, BW, BH, VEIL_GREEN);
  const char *btn = "Click me";
  veil_draw_text(BX + 30, BY + 11, btn, 8, VEIL_WHITE, 18);

  const char *lbl = "Clicks:";
  veil_draw_text(20, 164, lbl, 7, VEIL_TEXT, 18);
  char num[12];
  int len = int_to_str(clicks, num);
  veil_draw_text(96, 164, num, len, VEIL_GOLD, 18);
}

__attribute__((export_name("on_click")))
void on_click(int x, int y) {
  if (x >= BX && x < BX + BW && y >= BY && y < BY + BH) {
    clicks++;
    char msg[24];
    char *p = msg;
    const char *pre = "clicks=";
    for (int i = 0; i < 7; i++) *p++ = pre[i];
    p += int_to_str(clicks, p);
    veil_log(msg, (int)(p - msg));
  }
}
