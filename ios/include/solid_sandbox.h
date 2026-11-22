// The Rust core's C interface (src/ios.rs). All calls on the main thread.
#ifndef SOLID_SANDBOX_H
#define SOLID_SANDBOX_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct App SSApp;

// Make the app for a w x h pixel view; settings and palettes are kept in
// data_dir (UTF-8 path, or NULL). Returns NULL if it failed to start.
SSApp *ss_create(int32_t w, int32_t h, const char *data_dir);
void ss_destroy(SSApp *app);

// The view's new pixel size.
void ss_resize(SSApp *app, int32_t w, int32_t h);
// A finger going down at view pixel (x, y).
void ss_tap(SSApp *app, float x, float y);
// Frame-rate cap (frames per second), and whether gravity wants the accelerometer.
float ss_fps(SSApp *app);
bool ss_wants_accel(SSApp *app);
// Advance dt real seconds and render. With has_accel, (ax, ay, az) is
// CoreMotion's accelerometer reading in g and rotation is 0 portrait,
// 1 landscape right (top edge left), 2 upside down, 3 landscape left.
void ss_step(SSApp *app, float dt, bool has_accel, float ax, float ay, float az, int32_t rotation);
// Draw into h rows of stride pixels (bytes R, G, B, X), w used per row.
void ss_draw(SSApp *app, uint32_t *pixels, int32_t stride, int32_t w, int32_t h);

#ifdef __cplusplus
}
#endif

#endif