// wrapper.h
#include <ass/ass.h>

// 声明 wrapper.c 中定义的函数
void set_rust_message_callback(ASS_Library *priv, void *rust_callback_data);
void clear_message_callback(ASS_Library *priv);
