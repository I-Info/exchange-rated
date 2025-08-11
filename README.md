# Rated - 澳元汇率监控系统

一个用 Rust 编写的实时澳元汇率监控和展示系统，具有 WebSocket 实时更新和 SQLite 持久化存储功能。

## 功能特性

- 🔄 实时获取中国银行澳元汇率数据
- 📊 Web 界面展示历史汇率趋势
- 🔌 WebSocket 实时推送汇率更新
- 💾 SQLite 数据库持久化存储
- ⚡ 智能缓存机制，避免重复请求
- 📱 响应式设计，支持移动端

### 数据库结构
```sql
CREATE TABLE rate_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rate TEXT NOT NULL,
    timestamp DATETIME NOT NULL,
);

CREATE UNIQUE INDEX idx_rate_records_timestamp ON rate_records(timestamp);
```

## 安装和运行

### 前置要求
- Rust 1.80 或更高版本

### 克隆项目
```bash
git clone <repository-url>
cd rated
```

### 运行应用
```bash
cargo run
```

应用将在 `http://127.0.0.1:3000` 启动。

### 数据库文件
- SQLite 数据库文件: `rates.db`
- 迁移文件: `migrations/001_create_rate_records.sql`

## API 端点

### HTTP 端点
- `GET /` - 主页面，显示汇率历史图表
- `GET /api/latest` - 获取最新汇率数据
- `GET /api/history` - 获取历史汇率数据

### WebSocket 端点
- `ws://localhost:3000/ws` - 实时汇率更新推送

#### WebSocket 消息格式
```json
{
  "type": "rate_update",
  "record": {
    "rate": "5.0123",
    "timestamp": "2024-01-01T00:00:00Z"
  }
}
```

## 项目结构

```
rated/
├── src/
│   └── main.rs          # 主应用代码
├── migrations/
│   └── 001_create_rate_records.sql  # 数据库迁移
├── templates/
│   └── index.html       # HTML 模板
├── Cargo.toml           # 依赖配置
└── README.md            # 项目说明
```

## 技术栈

- **Web 框架**: Axum
- **异步运行时**: Tokio
- **数据库**: SQLite + SQLx
- **HTTP 客户端**: Reqwest
- **模板引擎**: Askama
- **WebSocket**: Axum WebSocket
- **序列化**: Serde
- **时间处理**: Chrono

## 监控和日志

应用提供实时控制台输出：
- `+` - 新汇率数据获取成功
- `.` - HTTP 304 响应（数据未更新）
- `*` - 错误或警告信息

## 许可证

本项目采用 MIT 许可证。
