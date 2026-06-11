/* Custom FreeType "standard library" config for no-OS AArch64 (Veil). */
#ifndef FT_VEIL_STDLIB_H
#define FT_VEIL_STDLIB_H
#include <stddef.h>
#include <stdint.h>
#include <limits.h>
#include <stdarg.h>
#define FT_CHAR_BIT  CHAR_BIT
#define FT_USHORT_MAX  USHRT_MAX
#define FT_INT_MAX   INT_MAX
#define FT_INT_MIN   INT_MIN
#define FT_UINT_MAX  UINT_MAX
#define FT_LONG_MIN  LONG_MIN
#define FT_LONG_MAX  LONG_MAX
#define FT_ULONG_MAX ULONG_MAX
extern void* memcpy(void*, const void*, size_t);
extern void* memmove(void*, const void*, size_t);
extern void* memset(void*, int, size_t);
extern int   memcmp(const void*, const void*, size_t);
extern void* memchr(const void*, int, size_t);
extern size_t strlen(const char*);
extern int   strcmp(const char*, const char*);
extern int   strncmp(const char*, const char*, size_t);
extern char* strcpy(char*, const char*);
extern char* strncpy(char*, const char*, size_t);
extern char* strcat(char*, const char*);
extern char* strrchr(const char*, int);
extern char* strstr(const char*, const char*);
extern long  strtol(const char*, char**, int);
extern void  veil_qsort(void*, size_t, size_t, int(*)(const void*, const void*));
#define ft_memchr  memchr
#define ft_memcmp  memcmp
#define ft_memcpy  memcpy
#define ft_memmove memmove
#define ft_memset  memset
#define ft_strcat  strcat
#define ft_strcmp  strcmp
#define ft_strcpy  strcpy
#define ft_strlen  strlen
#define ft_strncmp strncmp
#define ft_strncpy strncpy
#define ft_strrchr strrchr
#define ft_strstr  strstr
#define ft_qsort   veil_qsort
#define ft_strtol  strtol
typedef struct { uint64_t buf[24]; } veil_jmp_buf[1];
#define ft_jmp_buf veil_jmp_buf
extern int  veil_setjmp(void*);
extern void veil_longjmp(void*, int) __attribute__((noreturn));
#define ft_setjmp(b)     veil_setjmp((void*)&(b))
#define ft_longjmp(b, v) veil_longjmp((void*)&(b), (v))
#define ft_ptrdiff_t ptrdiff_t
extern void* veil_malloc(size_t);
extern void  veil_free(void*);
extern void* veil_realloc(void*, size_t);
extern void* veil_calloc(size_t, size_t);
#define ft_scalloc  veil_calloc
#define ft_sfree    veil_free
#define ft_smalloc  veil_malloc
#define ft_srealloc veil_realloc
#define ft_getenv( x )  ((char*)0)
#endif
