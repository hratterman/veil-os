/* Freestanding C-runtime functions FreeType needs that compiler-builtins
   doesn't provide (mem* come from Rust's compiler_builtins). */
#include <stddef.h>

size_t strlen(const char* s){ const char* p=s; while(*p) p++; return (size_t)(p-s); }
int strcmp(const char* a, const char* b){ while(*a && *a==*b){a++;b++;} return (unsigned char)*a-(unsigned char)*b; }
int strncmp(const char* a, const char* b, size_t n){ while(n && *a && *a==*b){a++;b++;n--;} return n? (unsigned char)*a-(unsigned char)*b : 0; }
char* strcpy(char* d, const char* s){ char* r=d; while((*d++=*s++)); return r; }
char* strncpy(char* d, const char* s, size_t n){ char* r=d; while(n && (*d=*s)){d++;s++;n--;} while(n--) *d++=0; return r; }
char* strcat(char* d, const char* s){ char* r=d; while(*d) d++; while((*d++=*s++)); return r; }
char* strrchr(const char* s, int c){ const char* last=0; do{ if(*s==(char)c) last=s; }while(*s++); return (char*)last; }
char* strstr(const char* h, const char* n){ if(!*n) return (char*)h; for(;*h;h++){ const char *a=h,*b=n; while(*a&&*b&&*a==*b){a++;b++;} if(!*b) return (char*)h; } return 0; }
long strtol(const char* s, char** end, int base){
    long r=0; int neg=0; while(*s==' '||*s=='\t') s++;
    if(*s=='-'){neg=1;s++;} else if(*s=='+') s++;
    if(base==0){ if(s[0]=='0'&&(s[1]=='x'||s[1]=='X')){base=16;s+=2;} else if(s[0]=='0'){base=8;s++;} else base=10; }
    for(;;){ int d; char c=*s; if(c>='0'&&c<='9') d=c-'0'; else if(c>='a'&&c<='z') d=c-'a'+10; else if(c>='A'&&c<='Z') d=c-'A'+10; else break; if(d>=base) break; r=r*base+d; s++; }
    if(end) *end=(char*)s; return neg?-r:r;
}
/* Simple insertion+quicksort-free qsort (FreeType sorts tiny arrays). */
void veil_qsort(void* base, size_t n, size_t sz, int(*cmp)(const void*, const void*)){
    char* a=(char*)base; char tmp[256];
    if(sz>sizeof(tmp)) return;
    for(size_t i=1;i<n;i++){ for(size_t j=i; j>0 && cmp(a+(j-1)*sz, a+j*sz)>0; j--){
        __builtin_memcpy(tmp, a+(j-1)*sz, sz); __builtin_memcpy(a+(j-1)*sz, a+j*sz, sz); __builtin_memcpy(a+j*sz, tmp, sz); } }
}
void* memchr(const void* s, int c, size_t n){ const unsigned char* p=s; while(n--){ if(*p==(unsigned char)c) return (void*)p; p++; } return 0; }
