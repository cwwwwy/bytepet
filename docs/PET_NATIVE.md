# Codex 原生宠物复刻说明

这份文档记录「BytePet 要和 Codex 自带宠物一致」这件事的实测依据，以及每条
行为的实现位置。所有结论都来自对真实宠物包（`~/.codex/pets/boba`，Codex
发行的 V2 宠物）逐帧测量，而不是凭印象写的。

## 1. 测量工具

```powershell
cargo run -p bytepet-core --example pet_inspect -- <宠物目录> [输出目录]
```

它会打印：

- 清单声明的 `spriteVersionNumber`、图片尺寸、解析出的网格与单元格；
- 每一行里真正画了内容的列（`row occupancy`）；
- 引擎最终解析出的动画表（行号、帧数、是否循环、回落状态）。

带输出目录时会按行导出 PNG（`row-00.png` … `row-10.png`），便于逐帧核对。

## 2. 实测结论（V2 / 8×11）

发行版宠物 `boba` 的实测结果：

| 行 | 状态 | 画面内容 | 实测帧数 |
|---:|---|---|---:|
| 0 | `idle` | 眨眼、抿一口奶茶 | **7**（官方表写 6） |
| 1 | `running-right` | 向右小跑 | 8 |
| 2 | `running-left` | 向左小跑 | 8 |
| 3 | `waving` | 挥手 | 4 |
| 4 | `jumping` | 跳一下 | 5 |
| 5 | `failed` | 泄气/低落 | 8 |
| 6 | `waiting` | 安静等待 | 6 |
| 7 | `running` | 工作、上下弹跳 | 6 |
| 8 | `review` | 喝奶茶、收尾 | 6 |
| 9 | `look-row-9` | 原地转一圈（转向右侧） | 8 |
| 10 | `look-row-10` | 原地转一圈（转向左侧） | 8 |

两个重要发现：

1. **帧数不能照抄官方表**。官方时长表里 `idle` 是 6 帧，但发行版宠物画了
   7 帧。引擎现在以图集里“真正有像素的格子”为准（`PetAtlas::occupancy`），
   再把官方时长模式拉伸/裁剪到该帧数：多出来的帧沿用中间帧时长，最后一帧
   仍然保留那个更长的停顿。`adapt_durations` 有对应单元测试。
2. **第 9、10 行不是静态的左看/右看**，而是一整段“转过去再转回来”的循环。
   所以它们被实现成**一次性动作（glance）**：鼠标越过宠物一侧时播一遍，
   播完回到 base，而不是一直定格在侧面。

### 行 9 / 行 10 到底朝哪边

靠肉眼判断容易搞反，所以用了一个可验证的测量：取头部区域内“深色像素
（眼睛、鼻子）”的水平重心，减去头部轮廓的水平中心。

实测（数值为正表示朝屏幕右侧）：

| 行 | 第 0 帧 | 中间帧 | 第 7 帧 |
|---|---:|---:|---:|
| 1 `running-right` | +6 | +7 | +8 |
| 2 `running-left` | −8 | −6 | −6 |
| 9 `look-row-9` | 0 | **+9…+11** | +1 |
| 10 `look-row-10` | 0 | **−4…−9** | 0 |

先用第 1、2 行（已知方向的跑动）验证了这个指标，再据此确定：
**第 9 行 = 转向右侧，第 10 行 = 转向左侧**。
实现见 `PetState::look_towards_right` / `look_towards_left` / `look_towards`。

## 3. 复刻了哪些行为

| Codex 宠物行为 | BytePet 实现 |
|---|---|
| 全部 11 行动画与官方时长 | `crates/bytepet-core/src/pet/state.rs`（`official_durations` + 占用度校正） |
| V1(8×9) / V2(8×11) 自动识别 | `PetManifest::resolve_frame` + `PetLibrary::load_entry` 读图片头推断 |
| 鼠标在宠物两侧时转头看 | `BytePetApp::update_glance` + `PetEngine::glance`（一次性，带 0.9s 冷却与死区） |
| 左键：打招呼（挥手 + 气泡） | `on_pet_click` → `trigger_greeting("click")`，DeepSeek 不可用时回落到人格里的固定/时段问候 |
| 双击：跳一下 | `on_double_click`（320ms 内第二次点击才判定为双击，不会误触发单击） |
| 拖拽移动 | 窗口 `ViewportCommand::StartDrag` |
| 右键菜单 | 窗口内 `egui::Area` 菜单（打开设置 / 关闭宠物），位置按窗口边界收拢 |
| 像素级点击穿透 | `AlphaMask` + `opaque_at_cell_dilated`（1 格≈4px 外扩，避免抗锯齿边缘点不中） |
| 自动行走（活动提醒） | `update_auto_walk`：默认 45 分钟一次，走 8 秒、速度 18px/s、范围 120px，可在设置里调；有事件/问候时不打断 |
| 换宠物 | 设置 →「宠物」：扫描 `~/.codex/pets`、`~/.unipet/pets` 与本地库，带首帧预览，切换后热替换图集与动画并写回配置 |
| 状态驱动（Codex hooks） | `crates/bytepet-core/src/state_server.rs` 本地 HTTP 协议，见下节 |

## 4. 本地状态协议

默认监听 `127.0.0.1:17872`（设置 →「状态协议」里可改端口或关闭）。协议与旧版
BytePet / UniPet 相同，任何脚本都能驱动：

```bash
curl -XPOST http://127.0.0.1:17872/state \
  -H 'content-type: application/json' \
  -d '{"source":"codex","state":"running","message":"正在跑测试","ttlMs":120000}'
```

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/state` | 接受事件返回 `202 {"ok":true}`；状态名非法返回 `400` |
| `GET` | `/health` | 当前宠物、人格、状态与宠物库快照 |
| `GET` | `/pets` | 宠物 id 列表 |

可用状态名：`idle`、`running`(=`working`/`thinking`)、`waiting`(=`blocked`)、
`failed`(=`error`)、`review`(=`success`/`done`)、`waving`(=`hello`)、`jumping`、
`running-left`、`running-right`、`look-row-9`、`look-row-10`。

`ttlMs: 0` 或省略表示不过期；带 `ttlMs` 的状态到点后自动回到基础动画
（`PetEngine::tick`，由 `BytePetApp::update_pet_timers` 每帧驱动）。

## 5. 逐帧验收方式

1. `cargo run -p bytepet-core --example pet_inspect -- <pet> out` 导出各行 PNG；
2. 对比 `docs/PET_NATIVE.md` 第 2 节的表格，确认行内容与帧数；
3. 在应用里逐条触发：左键（挥手+气泡）、双击（跳）、右键（菜单）、鼠标越过
   左右两侧（转头）、等自动行走、托盘显示/隐藏、设置里换宠物；
4. 用上面的 `curl` 推一个 `waiting`/`failed`/`review`，确认宠物切换到对应行
   并在 TTL 结束后回到 idle。
