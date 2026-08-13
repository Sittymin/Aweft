# AI 模型测试基准

本分支用于评估 AI 模型定位与修复 Slint UI 缺陷的能力。每个任务都从"已复现 bug、未含修复"的干净基线开始，评价模型能否自行定位根因并修复。

## 测试流程

1. 从本分支新建临时工作区，每个模型独立一个：
   ```bash
   git worktree add -b <model> /tmp/<model> HEAD
   ```
2. 运行 GUI 复现现象：
   ```bash
   cd /tmp/<model>
   CARGO_TARGET_DIR=<主仓库>/target nix develop --command cargo run
   ```
3. 把任务描述（见下方「题目」）原样交给模型，约束它只查看当前工作区代码。
4. 回收工作区：`git worktree remove --force /tmp/<model>` 并删除对应分支。

> 注意：子模块需先初始化并生成 configure 才能构建：
> ```bash
> git -C /tmp/<model> submodule update --init aweft-libass/third_party/libass
> cd /tmp/<model> && nix develop --command sh -c 'cd aweft-libass/third_party/libass && ./autogen.sh'
> ```

## 题目：编辑器左侧搜索框不显示

> 曲线编辑器左侧面板（`ChannelPanel`）和 NLA 左侧面板（`NlaPanel`）顶部的搜索输入框完全不显示——本应在面板左上角、与顶部时间标尺同一行出现一个带放大镜图标和占位文字（"搜索"/"搜索轨道"）的搜索框，但打开编辑器后该位置是空的，整个搜索框（含图标、占位提示）都看不到。
>
> 预期：搜索框显示在面板左上角、与标尺同层，下方通道/轨道列表第一行与画布第一泳道对齐。
>
> 相关代码：`aweft-gui/ui/widgets/editor_search_bar.slint` 及其调用处。
>
> 约束：只允许查看当前工作区代码，禁止查看 git 历史或其它分支。

> 参考答案不随本分支提交（避免泄题）。判定/参考答案维护在主干分支或线下。

