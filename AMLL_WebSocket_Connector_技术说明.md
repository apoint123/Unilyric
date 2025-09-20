# AMLL Connector WebSocket 连接技术说明

## 概述
AMLL Connector 是一个专业的 WebSocket 连接管理器，专门用于处理音乐播放器和歌词显示系统之间的实时通信。采用 Rust 语言开发，基于 Tokio 异步运行时和 tokio-tungstenite WebSocket 库。

## 架构设计

### 核心组件
```
Unilyric/src/amll_connector/
├── worker.rs          # Actor 系统和连接状态管理
├── websocket_client.rs # WebSocket 客户端实现
├── protocol.rs        # 消息协议定义
├── translation.rs     # 数据转换工具
└── types.rs          # 类型定义
```

### 连接状态机 (worker.rs:52-64)
```rust
enum ConnectionState {
    Disconnected,                    // 未连接状态
    WaitingToRetry(Pin<Box<Sleep>>), // 等待重连计时器
    Running {                        // 活跃连接状态
        tx: TokioSender<OutgoingMessage>,
        shutdown_tx: oneshot::Sender<()>,
        handle: JoinHandle<anyhow::Result<()>>,
    },
    ShuttingDown {                   // 关闭中状态
        handle: JoinHandle<anyhow::Result<()>>,
        next_action: PostShutdownAction,
    },
}
```

## 协议设计

### 消息格式
支持两种消息格式，确保向后兼容：

#### JSON 协议 (protocol.rs:62-96)
```rust
// 客户端到服务器消息
enum ClientMessage {
    InitializeV2,
    Ping,
    Pong,
    SetMusicInfo { /* 歌曲元数据 */ },
    OnPlayProgress { progress: u64 },
    OnPaused,
    OnResumed,
    SetLyric { data: Vec<LyricLine> },
}

// 服务器到客户端消息
enum ServerMessage {
    Ping,
    Pong,
    Pause,
    Resume,
    ForwardSong,
    BackwardSong,
    SetVolume { volume: f64 },
    SeekPlayProgress { progress: u64 },
}
```

#### 二进制协议 (protocol.rs:101-169)
使用 binrw 库进行高效的二进制序列化，支持：
- 音频数据传输
- 专辑封面图片
- 高效的歌词传输

## 连接生命周期管理

### 1. 连接建立流程
```rust
// worker.rs:154-185
fn start_websocket_client_task() -> Result<ClientTaskComponents> {
    // 1. URL 验证 (ws:// 或 wss://)
    // 2. 创建通信通道 (32缓冲大小)
    // 3. 创建关闭信号通道
    // 4. 启动 WebSocket 客户端任务
}
```

### 2. 连接处理循环 (websocket_client.rs:300-389)
```rust
async fn handle_connection() -> Result<(), LifecycleEndReason> {
    loop {
        tokio::select! {
            // 1. 外部关闭信号处理
            // 2. 待发送消息处理 (来自Actor)
            // 3. WebSocket 服务器消息处理
            // 4. 应用层心跳定时器
        }
    }
}
```

## 关键技术特性

### 1. 自动重连机制 (worker.rs:483-506)
- **指数退避策略**: 5秒 → 10秒 → 20秒
- **最大重试次数**: 3次后停止自动重连
- **状态通知**: 实时更新连接状态到UI

### 2. 心跳检测系统 (websocket_client.rs:364-386)
```rust
const APP_PING_INTERVAL: Duration = Duration::from_secs(5);
const APP_PONG_TIMEOUT: Duration = Duration::from_secs(5);

// 应用层 Ping/Pong 机制
// 超时自动断开连接
```

### 3. 防抖和节流机制
```rust
const SEEK_DEBOUNCE_DURATION: Duration = Duration::from_millis(500);
const MIN_VOLUME_SET_INTERVAL: Duration = Duration::from_millis(100);
const AUDIO_SEND_INTERVAL: Duration = Duration::from_millis(10);
```

### 4. 错误处理体系
```rust
enum LifecycleEndReason {
    PongTimeout,                    // 心跳超时
    StreamFailure(String),          // 流错误
    ServerClosed,                   // 服务器关闭
}
```

## 数据流处理

### 从播放器到WebSocket服务器
1. **歌曲元数据** - 标题、艺术家、专辑、时长
2. **播放状态** - 播放/暂停状态同步
3. **进度更新** - 实时播放进度
4. **音频数据** - 频谱分析数据 (f32 → i16转换)
5. **专辑封面** - 图片二进制数据

### 从WebSocket服务器到播放器
1. **播放控制** - 播放/暂停/上一首/下一首
2. **进度跳转** - 定位到特定时间点
3. **音量控制** - 调整播放音量
4. **心跳响应** - Ping/Pong 维护连接

## 性能优化

### 内存管理
- **通道缓冲**: 固定大小32的MPSC通道
- **零拷贝**: 尽可能避免数据复制
- **类型优化**: 使用高效的数据结构

### 网络优化
- **二进制协议**: 减少序列化开销
- **批量处理**: 音频数据节流发送
- **连接复用**: 保持长连接减少握手开销

## 监控和日志

集成 tracing 库，提供详细的日志输出：
```bash
# 设置日志级别
RUST_LOG=lyrics_helper_rs=debug
```

日志级别包括:
- `error` - 连接错误和严重问题
- `warn` - 警告信息 (如通道满)
- `info` - 重要状态变化
- `debug` - 详细调试信息
- `trace` - 最详细的跟踪信息

## 安全考虑

1. **URL 验证**: 强制 ws:// 或 wss:// 协议
2. **超时控制**: 连接和心跳超时保护
3. **数据验证**: 消息反序列化错误处理
4. **资源限制**: 防止内存泄漏和DoS攻击

## 扩展性

### 协议扩展
- 支持新旧协议版本共存
- 易于添加新的消息类型
- 二进制和JSON格式并行支持

### 功能扩展
- 支持更多媒体控制命令
- 可扩展的音视频数据处理
- 插件式提供商支持

## 部署和使用

### 配置参数
```rust
struct AMLLConnectorConfig {
    enabled: bool,
    websocket_url: String,
    // 其他配置项...
}
```

### 启动命令
```bash
# 启用连接器
cargo run --release -p Unilyric
```

## 故障排除

### 常见问题
1. **连接失败**: 检查URL格式和网络连通性
2. **心跳超时**: 检查服务器响应时间
3. **消息丢失**: 检查通道缓冲状态
4. **性能问题**: 调整节流参数

### 调试技巧
```rust
// 启用详细日志
RUST_LOG=lyrics_helper_rs=debug,amll_connector=debug
```

---

*本系统为 UniLyric 项目的核心组件，提供稳定可靠的实时通信能力。*