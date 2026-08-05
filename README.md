# ImgGen

ImgGen 是一个使用 **Rust + Slint** 构建的 Windows 桌面图片生成工作台，同时提供标准 **MCP stdio Server**。它可以连接 OpenAI-compatible 图片接口，获取模型、生成图片、保存历史记录，并让 Codex、Claude、Cursor、VS Code 等 MCP 客户端调用同一套图片生成和历史管理能力。

> 当前 Release 首版面向 Windows x86_64。项目使用 MIT License。

## 功能概览

- Q 版 ImgGen 桌面界面和 ImgGen 图标。
- Windows GUI 双击启动时不显示控制台窗口；任务栏、开始菜单和桌面快捷方式使用 ImgGen 图标。
- 支持多个 API 提供商。
- 支持保存、复制、删除和切换 API 提供商。
- 支持从 `/models` 获取模型列表，也可以手动填写模型。
- 支持模型滚动选择。
- 支持提示词、分辨率、格式和数量配置。
- 支持浅色/深色主题和主题色取色板。
- 兼容常见 OpenAI-compatible 图片接口：
  - `GET /models`
  - `POST /images/generations`
- 支持图片接口返回 `b64_json` 或 `url`。
- 生成成功后将每张图片保存到应用目录。
- 历史记录可以恢复预览；删除历史记录时会同时删除关联图片。
- 支持 MCP stdio Server：
  - 生成图片
  - 查询历史
  - 删除历史和图片
  - 查询、保存、删除、切换 API 提供商
  - 获取 API 提供商模型列表

## 下载与安装

GitHub Release 页面：

<https://github.com/ling552/imgGenMCP/releases>

每个 Windows Release 提供两个文件：

- `ImgGen-windows-x86_64-vX.Y.Z-setup.exe`：Windows 安装程序。
- `ImgGen-windows-x86_64-vX.Y.Z.zip`：便携版压缩包。

### 使用安装程序

1. 下载 `setup.exe`。
2. 双击运行安装程序。
3. 默认安装目录为：

   ```text
   %LOCALAPPDATA%\Programs\ImgGen
   ```

4. 安装程序默认不需要管理员权限。
5. 安装完成后从开始菜单或桌面快捷方式启动 ImgGen。

使用用户可写安装目录是有意设计：ImgGen 默认把配置和历史图片放在 exe 同目录的 `data` 文件夹中，安装到 `Program Files` 可能导致普通用户没有写入权限。

卸载程序不会删除 `data` 中的供应商配置、API Key、历史记录和图片。确认不再需要数据后，可以手动删除安装目录中的 `data` 文件夹。

### 使用便携版

1. 解压 ZIP 到一个有写入权限的目录，例如：

   ```text
   C:\Users\<用户名>\Apps\ImgGen
   ```

2. 运行目录中的 `imggen.exe`。
3. 不要把便携版放到需要管理员权限的目录，否则配置和图片可能无法保存。

## 首次配置

1. 启动 ImgGen。
2. 点击右下角设置按钮。
3. 在“供应商”页新增或选择 API 提供商。
4. 填写：
   - 提供商名称
   - Base URL
   - API Key
   - 模型
5. 点击“获取模型”，或在无法获取模型时手动填写模型。
6. 保存 API 提供商设置。
7. 返回主界面选择模型，填写提示词后点击“生成图片”。

### Base URL 规则

ImgGen 会在 Base URL 后拼接接口路径，不会自动猜测或追加 `/v1`。请按照供应商实际要求填写，例如：

```text
https://api.openai.com/v1
```

最终生图请求会访问：

```text
https://api.openai.com/v1/images/generations
```

如果供应商使用其他前缀，请按实际 API 文档填写 Base URL。

## 数据目录

ImgGen 使用运行中的 `imggen.exe` 所在目录作为应用根目录，并在其中创建 `data`：

```text
data/
├─ history.json       # 历史记录元数据
├─ providers.json     # API 提供商配置
├─ images/            # 已生成图片
└─ .storage.lock      # GUI 与 MCP 进程之间的文件锁
```

历史记录中的图片文件名由程序生成，不使用提示词拼接路径。删除历史记录时会同时删除 `data/images/` 中对应的图片。

备份 ImgGen 数据时请整体备份 `data/`，不要只备份 `history.json`。

### API Key 存储安全说明

当前版本按照项目配置直接把 API Key 保存在：

```text
data/providers.json
```

该文件是明文 JSON，不是 Windows Credential Manager 加密存储。请注意：

- 不要把 `data/` 上传到 GitHub。
- 不要把 `providers.json` 发送给他人。
- 不要在公开 Issue、README、截图或日志中暴露 API Key。
- 如果怀疑密钥泄露，请立即在供应商后台撤销并重新生成。
- GitHub Actions 和本仓库示例不会包含任何真实 API Key。

## MCP Server

ImgGen 的 MCP Server 使用标准 **stdio transport**。MCP 客户端启动以下进程即可连接：

```text
C:\Users\<用户名>\AppData\Local\Programs\ImgGen\imggen.exe --mcp
```

路径包含空格时必须加双引号。MCP 模式下：

- stdout 只用于 MCP JSON-RPC 消息。
- 不要在启动命令前后加入 `echo`、普通日志或其他文本。
- 数据目录仍然以 `imggen.exe` 所在目录为准，不依赖 MCP 客户端的当前工作目录。
- MCP 使用 `data/providers.json` 中的当前 API 提供商。
- MCP 返回结果不会包含 API Key。
- 生成图片、历史记录和 GUI 使用同一个 `data` 目录。

### MCP 工具列表

#### `generate_image`

生成图片并保存历史记录。

必填参数：

```json
{
  "prompt": "一只在月球上散步的 Q 版小猫"
}
```

可选参数：

```json
{
  "prompt": "一只在月球上散步的 Q 版小猫",
  "resolution": "1024 × 1024",
  "image_format": "PNG",
  "quantity": 2,
  "provider": "我的 API"
}
```

`quantity` 范围为 1 到 10。每张图片会建立一条独立历史记录，并返回本地图片路径。

#### `list_history`

查询历史记录和图片路径，不需要参数：

```json
{}
```

#### `delete_history`

按历史 ID 删除记录及关联图片：

```json
{
  "id": "历史记录 ID"
}
```

#### `list_providers`

查询已配置的 API 提供商。返回内容不包含 `api_key`。

#### `save_provider`

保存或更新 API 提供商：

```json
{
  "name": "我的 API",
  "base_url": "https://api.example.com/v1",
  "api_key": "YOUR_API_KEY",
  "model": "image-model",
  "models": ["image-model"]
}
```

更新已有提供商时可以省略 `api_key`，这样会保留原有 API Key。不要把真实密钥写入公开配置文件或提交到仓库。

#### `delete_provider`

按名称删除 API 提供商。至少需要保留一个提供商：

```json
{
  "name": "我的 API"
}
```

#### `select_provider`

切换当前提供商：

```json
{
  "name": "我的 API"
}
```

#### `fetch_provider_models`

从指定提供商的 `/models` 获取模型并保存：

```json
{
  "name": "我的 API"
}
```

## Codex CLI 配置

Codex 支持使用命令行添加本地 stdio MCP Server。

### 命令行添加

PowerShell 示例：

```powershell
codex mcp add imggen -- "C:\Users\<用户名>\AppData\Local\Programs\ImgGen\imggen.exe" --mcp
```

如果路径包含空格，保留外层双引号。

查看配置和连接状态：

```powershell
codex mcp list
codex mcp --help
```

进入 Codex TUI 后也可以使用：

```text
/mcp
```

### `config.toml` 配置

Codex 用户级配置通常位于：

```text
%USERPROFILE%\.codex\config.toml
```

配置格式与 Claude Desktop 不同，使用 `mcp_servers`：

```toml
[mcp_servers.imggen]
command = 'C:\Users\<用户名>\AppData\Local\Programs\ImgGen\imggen.exe'
args = ['--mcp']
startup_timeout_sec = 20
tool_timeout_sec = 180
enabled = true
```

也可以使用项目级配置：

```text
<项目目录>\.codex\config.toml
```

仅在确认项目可信时使用项目级 MCP 配置。不要把含 API Key 的 `data/` 放进项目仓库。

## Claude Code 配置

Claude Code 使用 `claude mcp add` 管理本地 stdio Server。

### 命令行添加

```powershell
claude mcp add --transport stdio imggen -- "C:\Users\<用户名>\AppData\Local\Programs\ImgGen\imggen.exe" --mcp
```

注意命令中的 `--`：它用于分隔 Claude Code 自身参数和传给 ImgGen 的参数。

查看、获取和删除配置：

```powershell
claude mcp list
claude mcp get imggen
claude mcp remove imggen
```

进入 Claude Code 后查看连接状态和工具数量：

```text
/mcp
```

### 项目级 `.mcp.json`

如果希望只对某个项目启用，可以在项目根目录创建 `.mcp.json`：

```json
{
  "mcpServers": {
    "imggen": {
      "type": "stdio",
      "command": "C:\\Users\\<用户名>\\AppData\\Local\\Programs\\ImgGen\\imggen.exe",
      "args": ["--mcp"]
    }
  }
}
```

Claude Code 对项目级 MCP 配置可能要求工作区信任或人工批准。连接后运行 `claude mcp list` 或在会话中使用 `/mcp` 检查状态。

## Claude Desktop 配置

Claude Desktop 使用 `mcpServers` 字段，不是 VS Code/Cursor 使用的 `servers` 字段。

Windows 配置文件：

```text
%APPDATA%\Claude\claude_desktop_config.json
```

在 Claude Desktop 的设置中打开 Developer → Edit Config，也可以直接编辑该文件。

配置示例：

```json
{
  "mcpServers": {
    "imggen": {
      "command": "C:\\Users\\<用户名>\\AppData\\Local\\Programs\\ImgGen\\imggen.exe",
      "args": ["--mcp"]
    }
  }
}
```

保存后完全退出并重新启动 Claude Desktop。然后在输入框附近的连接器/工具入口查看 `imggen` 和它提供的工具。

Claude Desktop 的 MCP 日志通常位于：

```text
%APPDATA%\Claude\logs
```

如果没有连接：

1. 检查 JSON 反斜杠是否写成 `\\`。
2. 检查 exe 路径是否存在。
3. 在 PowerShell 中手动启动 MCP 并发送协议请求。
4. 检查 `mcp.log` 和对应的 MCP Server 日志。
5. 完全退出 Claude Desktop 后重新打开。

## Cursor 配置

Cursor 支持项目级和用户级 MCP 配置：

- 项目级：`.cursor/mcp.json`
- 用户级：`%USERPROFILE%\.cursor\mcp.json`

Cursor 使用 `servers`，并建议明确写入 `type: "stdio"`：

```json
{
  "servers": {
    "imggen": {
      "type": "stdio",
      "command": "C:\\Users\\<用户名>\\AppData\\Local\\Programs\\ImgGen\\imggen.exe",
      "args": ["--mcp"]
    }
  }
}
```

验证方法：

1. 在 Cursor 的 MCP/Customize 页面确认 `imggen` 已启用。
2. 在聊天窗口的 Available Tools 中检查 ImgGen 工具。
3. 打开 Output 面板并选择 MCP Logs，查看启动或协议错误。

## VS Code 配置

VS Code 使用 `mcp.json`，可以使用项目级或用户级配置：

- 项目级：`.vscode/mcp.json`
- 用户级：通过命令面板执行 `MCP: Open User Configuration`

VS Code 配置示例：

```json
{
  "servers": {
    "imggen": {
      "type": "stdio",
      "command": "C:\\Users\\<用户名>\\AppData\\Local\\Programs\\ImgGen\\imggen.exe",
      "args": ["--mcp"]
    }
  }
}
```

验证方法：

1. 在 VS Code 的 Chat/MCP 相关界面确认 Server 已启动。
2. 检查可用工具列表中是否出现 `generate_image`、`list_history` 等工具。
3. 查看 Output 面板中的 MCP 日志。
4. Windows 当前没有 VS Code MCP 沙箱能力，应该只配置可信的本地 Server。

## 其他 MCP 客户端

大多数支持本地 stdio 的 MCP 客户端都使用下面的概念：

```json
{
  "servers": {
    "imggen": {
      "type": "stdio",
      "command": "C:\\Users\\<用户名>\\AppData\\Local\\Programs\\ImgGen\\imggen.exe",
      "args": ["--mcp"]
    }
  }
}
```

但不同客户端的顶层字段可能不同：

| 客户端 | 顶层字段 | 常见配置位置 |
| --- | --- | --- |
| Claude Desktop | `mcpServers` | `%APPDATA%\Claude\claude_desktop_config.json` |
| Claude Code | `mcpServers` 或 `claude mcp add` | `.mcp.json` / `claude mcp` 配置 |
| Codex | `[mcp_servers.<name>]` | `%USERPROFILE%\.codex\config.toml` |
| Cursor | `servers` | `.cursor/mcp.json` 或 `%USERPROFILE%\.cursor\mcp.json` |
| VS Code | `servers` | `.vscode/mcp.json` 或用户配置 |

不要直接把 Claude Desktop 的 `mcpServers` 配置复制到 Codex、Cursor 或 VS Code；先按照对应客户端的字段格式转换。

## 手动验证 MCP

在 PowerShell 中直接启动 MCP：

```powershell
$exe = "C:\Users\<用户名>\AppData\Local\Programs\ImgGen\imggen.exe"
'{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"1"}}}' |
  & $exe --mcp
```

正常情况下 stdout 会返回 JSON-RPC 初始化结果。更完整地验证工具列表：

```powershell
@'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"1"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
'@ | & $exe --mcp
```

验证时不要把普通文本写入 stdin，也不要把调试日志写入 stdout。未知工具、参数错误和业务失败应该通过 MCP 标准响应返回。

## 故障排查

### GUI 无法保存配置或历史

- 确认 exe 所在目录可写。
- 避免安装到 `C:\Program Files`。
- 检查 `data` 是否被安全软件锁定。
- 确认 `data/providers.json` 和 `data/history.json` 是有效 JSON。
- 备份后再处理损坏的配置文件，不要直接删除仍需要的历史图片。

### 生图请求失败

状态区域会尽量显示：

- HTTP 状态码。
- 服务端错误正文。
- `Retry-After`。
- 超时、连接失败和底层网络原因。

请检查 Base URL、API Key、模型名称、分辨率、供应商额度和供应商是否支持图片接口。ImgGen 不会自动重试生图请求，避免重复生成和重复计费。

### MCP Server 未连接

按以下顺序检查：

1. 直接确认文件存在：

   ```powershell
   Test-Path "C:\Users\<用户名>\AppData\Local\Programs\ImgGen\imggen.exe"
   ```

2. 手动运行：

   ```powershell
   & "C:\Users\<用户名>\AppData\Local\Programs\ImgGen\imggen.exe" --mcp
   ```

3. 检查客户端配置中的路径转义。
4. 确认 args 是 `["--mcp"]`，不是把 `--mcp` 拼进 command 字符串。
5. 查看对应客户端的 MCP 日志。
6. 重启客户端。
7. 如果 GUI 与 MCP 同时运行，确认二者使用的是同一个 exe 目录和 `data` 目录。

### MCP 输出不是 JSON

MCP stdio 要求 stdout 只输出协议消息。如果在 stdout 中看到普通日志：

- 不要在启动命令前后添加 `echo`。
- 不要通过脚本把普通输出管道到 stdout。
- 不要修改程序把调试信息写入 stdout。
- 将调试信息写入 stderr，或关闭调试输出。

## 从源码构建

要求：

- Rust stable。
- Cargo。
- Windows x86_64 MSVC 工具链。

检查工具链：

```powershell
rustc --version
cargo --version
rustup target list --installed
```

构建和验证：

```powershell
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

启动 GUI：

```powershell
cargo run --release
```

启动 MCP：

```powershell
cargo run --release -- --mcp
```

## Release 构建

Windows 发布由 GitHub Actions 完成，工作流文件为：

```text
.github/workflows/release-windows.yml
```

推送匹配 `v*` 的版本标签后，Actions 会：

1. 安装 Windows x86_64 Rust 工具链。
2. 执行锁定依赖的 Release 构建。
3. 生成便携 ZIP。
4. 使用 Inno Setup 生成安装程序。
5. 创建或更新 GitHub Release 并上传两个资产。

本地 Inno Setup 定义位于：

```text
installer/imggen.iss
```

## 安全与功能边界

- API Key 当前以明文 JSON 保存，详见“数据目录”章节。
- 不自动重试生图请求。
- 不关闭 TLS 证书校验。
- 返回图片 URL 下载时不会把 API Key 发送到任意第三方域名。
- 当前提供 MCP stdio，不提供 HTTP MCP Server。
- 当前 Release 只提供 Windows x86_64 资产。
- 多张图片会分别建立历史记录，而不是保存为一个多图画廊记录。
- 项目不上传运行时 `data/`、`target/` 或真实凭据。

## License

本项目使用 [MIT License](LICENSE)。
