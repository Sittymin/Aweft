#include "wrapper.h"
#include <stdio.h>
#include <stdarg.h>
#include <stdlib.h>

// 外部声明：这个函数在 Rust 端实现
extern void rust_handle_message_callback(int level, const char *message, void *data);

void rust_message_callback_trampoline(int level, const char *fmt, va_list args, void *data) {
    va_list args_copy;
    va_copy(args_copy, args);

    int len = vsnprintf(NULL, 0, fmt, args_copy);
    
    va_end(args_copy);

    if (len < 0) {
        return;
    }

    char *buffer = (char *)malloc(len + 1);
    if (!buffer) {
        return;
    }

    int result = vsnprintf(buffer, len + 1, fmt, args);

    if (result < 0) {
        free(buffer);
        return;
    }

    rust_handle_message_callback(level, buffer, data);

    free(buffer);
}

// 设置 Rust 消息回调
void set_rust_message_callback(ASS_Library *priv, void *rust_callback_data) {
    ass_set_message_cb(priv, rust_message_callback_trampoline, rust_callback_data);
}

// 清除消息回调
void clear_message_callback(ASS_Library *priv) {
    ass_set_message_cb(priv, NULL, NULL);
} 
