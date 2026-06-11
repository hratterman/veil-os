/* Thin C glue so Rust only sees opaque handles + plain out-params. */
#include <ft2build.h>
#include <freetype/freetype.h>

int veil_ft_init(FT_Library* lib) {
    return FT_Init_FreeType(lib);
}

int veil_ft_new_face(FT_Library lib, const unsigned char* data, long size, FT_Face* face) {
    return FT_New_Memory_Face(lib, data, size, 0, face);
}

/* Render one glyph at `size_px`. On success returns 0 and fills the out-params;
   *out_buf points into FreeType's own 8-bit alpha bitmap (valid until the next
   load on this face, so the caller must copy it immediately). */
int veil_render_glyph(FT_Face face, unsigned long codepoint, unsigned int size_px,
                      const unsigned char** out_buf, int* w, int* rows, int* pitch,
                      int* left, int* top, int* advance) {
    if (FT_Set_Pixel_Sizes(face, 0, size_px)) return 1;
    if (FT_Load_Char(face, codepoint, FT_LOAD_RENDER | FT_LOAD_NO_HINTING)) return 2;
    FT_GlyphSlot g = face->glyph;
    *out_buf = g->bitmap.buffer;
    *w       = (int)g->bitmap.width;
    *rows    = (int)g->bitmap.rows;
    *pitch   = g->bitmap.pitch;
    *left    = g->bitmap_left;
    *top     = g->bitmap_top;
    *advance = (int)(g->advance.x >> 6);
    return 0;
}

#include <freetype/ftgzip.h>
#include <freetype/ftmm.h>
FT_Error FT_Gzip_Uncompress(FT_Memory memory, FT_Byte* output, FT_ULong* output_len,
                            const FT_Byte* input, FT_ULong input_len) {
    (void)memory;(void)output;(void)output_len;(void)input;(void)input_len;
    return FT_Err_Unimplemented_Feature;
}
FT_Error FT_Set_Named_Instance(FT_Face face, FT_UInt instance_index) {
    (void)face;(void)instance_index;
    return FT_Err_Ok;  /* no-op: default instance is fine */
}
