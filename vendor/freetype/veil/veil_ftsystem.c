/* Veil custom FreeType system layer: memory via the kernel heap (veil_*),
   and a stub file-stream opener (we only ever use FT_New_Memory_Face). */
#include <ft2build.h>
#include FT_CONFIG_CONFIG_H
#include <freetype/internal/ftdebug.h>
#include <freetype/internal/ftstream.h>
#include <freetype/ftsystem.h>
#include <freetype/fterrors.h>
#include <freetype/fttypes.h>

extern void* veil_malloc(unsigned long);
extern void  veil_free(void*);
extern void* veil_realloc(void*, unsigned long);

static FT_Pointer ft_alloc(FT_Memory memory, long size) {
    (void)memory;
    return veil_malloc((unsigned long)size);
}
static void ft_free(FT_Memory memory, FT_Pointer block) {
    (void)memory;
    veil_free(block);
}
static FT_Pointer ft_realloc(FT_Memory memory, long cur_size, long new_size, FT_Pointer block) {
    (void)memory; (void)cur_size;
    return veil_realloc(block, (unsigned long)new_size);
}

FT_BASE_DEF(FT_Memory) FT_New_Memory(void) {
    FT_Memory memory = (FT_Memory)veil_malloc(sizeof(*memory));
    if (memory) {
        memory->user    = NULL;
        memory->alloc   = ft_alloc;
        memory->realloc = ft_realloc;
        memory->free    = ft_free;
    }
    return memory;
}

FT_BASE_DEF(void) FT_Done_Memory(FT_Memory memory) {
    veil_free(memory);
}

FT_BASE_DEF(FT_Error) FT_Stream_Open(FT_Stream stream, const char* filepathname) {
    (void)stream; (void)filepathname;
    return FT_THROW(Cannot_Open_Resource);  /* no file I/O */
}
