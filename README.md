# jellycli


GeminiCLI to OpenAI API的rust实现
## 功能

- 将 Google Gemini API 转换为 OpenAI API 格式
- 支持流式和非流式响应
- 支持假流式响应
- 多凭证轮换机制
- 错误处理和冷却机制

## 配置

项目使用 `config.json` 文件进行配置。如果未找到配置文件，将使用默认设置。

### 配置文件示例

```json
{
  "password": "pwd",
  "bind_address": "0.0.0.0:7878",
  "credentials_dir": "./credentials",
  "code_assist_endpoint": "https://codeassist-pa.clients6.google.com",
  "calls_per_rotation": 100
}
```

### 配置项说明

- `password`: API 访问密码（默认：pwd）
- `bind_address`: 服务绑定地址（默认：0.0.0.0:7878）
- `credentials_dir`: Google 凭证文件目录（默认：./credentials）
- `code_assist_endpoint`: Gemini API 端点
- `calls_per_rotation`: 每个凭证的最大调用次数

## 凭证文件

将 Google OAuth 凭证文件放置在 `credentials` 目录中，文件格式应为 JSON：

```json
{
  "access_token": "your_access_token",
  "refresh_token": "your_refresh_token",
  "client_id": "your_client_id",
  "client_secret": "your_client_secret",
  "project_id": "your_project_id",
  "expiry": "2025-08-15T10:00:00Z"
}
```

## 支持的模型

- gemini-2.5-pro-preview-06-05
- gemini-2.5-pro-preview-06-05-假流式
- gemini-2.5-pro
- gemini-2.5-pro-假流式
- gemini-2.5-pro-preview-05-06
- gemini-2.5-pro-preview-05-06-假流式

## API 端点

- `/v1/chat/completions` - 聊天补全
- `/health` - 健康检查

## 使用方法

1. 复制 `config.example.json` 为 `config.json` 并根据需要修改配置
2. 将 Google 凭证文件放入 `credentials` 目录
3. 运行程序：
4. 使用 OpenAI 兼容的 API 访问：`http://localhost:7878/v1/chat/completions`