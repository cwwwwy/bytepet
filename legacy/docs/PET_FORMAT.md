# 宠物格式与 Codex 兼容性

本应用的目标是：**任何 Codex 桌宠包放进来就能用**，同时兼容社区扩展字段。

## 1. 包结构

一个宠物就是一个目录：

```
<pet-id>/
├── pet.json            # 清单（严格 UTF-8，无 BOM）
└── spritesheet.webp    # 图集（.webp / .png / .jpg）
```

应用会扫描以下目录（只读引用，不修改原文件）：

| 目录 | 来源 | 说明 |
|---|---|---|
| `~/Library/Application Support/com.bytepet.desktop/pets/`（macOS）<br>`%APPDATA%\com.bytepet.desktop\pets\`（Windows） | 本地库 | 通过「导入」复制进来的宠物 |
| `~/.codex/pets/*` | Codex | 直接使用你已有的 Codex 宠物 |
| `~/.unipet/pets/*` | UniPet | 兼容 UniPet 安装的宠物 |

同 id 时本地库优先。

## 2. `pet.json`

官方最小清单：

```json
{
  "id": "zhenzhu-xiaozi",
  "displayName": "珍珠小子",
  "description": "他的狗叫“鼠标”。",
  "spritesheetPath": "spritesheet.webp"
}
```

V2 增加注视行时带 `"spriteVersionNumber": 2`。

解析器同时接受这些字段（按优先级）：

| 字段 | 作用 |
|---|---|
| `id` | 必填，`[A-Za-z0-9._-]{1,64}`，不得为 `uni` |
| `displayName` / `name` | 显示名，缺省用 `id` |
| `description` | 描述 |
| `spritesheetPath` / `spritesheet` | 图集相对路径，缺省 `spritesheet.webp` |
| `spriteVersionNumber` | `1` → 8×9；`2` → 8×11 |
| `frame: {width,height,columns,rows}` | UniPet 风格显式几何 |
| `frameWidth` / `frameHeight` / `columns` / `rows` | 旧版字段 |
| `animations: {state: {frames, fps, loop, loopStart, fallback}}` | UniPet 风格动画覆盖 |
| 其它字段 | 原样保留，导出时不丢失 |

安全规则：

- `id` 必须是文件系统安全字符，`..`、绝对路径会被拒绝；
- `spritesheetPath` 不得包含 `..` 或以 `/` 开头（防目录穿越）；
- 图集文件 ≤ 16 MB；
- 图片尺寸必须能整除为整数个单元格。

## 3. 图集几何

| 版本 | 网格 | 单元格 | 整图 |
|---|---|---|---|
| V1（默认） | 8 列 × 9 行 | 192 × 208 | 1536 × 1872 |
| V2 | 8 列 × 11 行 | 192 × 208 | 1536 × 2288 |

若清单声明的几何与图片尺寸不符，但图片能按 192×208 整除，应用会**推断**出实际网格并给出警告（社区包经常忘记写 `spriteVersionNumber`）。

## 4. 行语义与官方帧时长

| 行 | 状态 | 帧数 | 每帧毫秒 |
|---:|---|---:|---|
| 0 | `idle` | 6 | 280, 110, 110, 140, 140, 320 |
| 1 | `running-right` | 8 | 120 ×7，最后一帧 220 |
| 2 | `running-left` | 8 | 120 ×7，最后一帧 220 |
| 3 | `waving` | 4 | 140, 140, 140, 280 |
| 4 | `jumping` | 5 | 140 ×4，最后一帧 280 |
| 5 | `failed` | 8 | 140 ×7，最后一帧 240 |
| 6 | `waiting` | 6 | 150 ×5，最后一帧 260 |
| 7 | `running`（思考/工作） | 6 | 120 ×5，最后一帧 220 |
| 8 | `review`（完成/检视） | 6 | 150 ×5，最后一帧 280 |
| 9 | `look-row-9` | 8 | 140 ×8（仅 V2） |
| 10 | `look-row-10` | 8 | 140 ×8（仅 V2） |

未使用的单元格应当完全透明；不透明时只警告、不报错。

如果清单里提供了 `animations`，对应状态以清单为准（`fps` 或逐帧 `durationMs`、`loop`、`fallback`），其余状态回落到官方表。

状态名兼容写法：`running_right`、`working`、`work`、`wave`、`success`、`error`、`look9` 等别名都会被识别（见 `bytepet-core/src/pet/state.rs`）。

## 5. 状态优先级

同时有多个来源时按优先级仲裁，高优先级覆盖低优先级，TTL 到期后回落到基础状态：

```
failed(90) > waiting(80) > running(70) > review(60)
          > waving/jumping(40) > 注视(20) > 行走(10) > idle(0)
```

- `waving` / `jumping` 是一次性动画，播完回到 `fallback`（默认 `idle`）；
- 自动行走把 `running-left` / `running-right` 作为**基础状态**，任何更高优先级事件都会让它暂停；
- 对话进行中为 `running`，等待用户确认为 `waiting`，出错为 `failed`，完成 3 秒 `review`。

## 6. 导入 / 导出

- 导入目录或 `.zip`：会先完整校验（清单、几何、解码、路径安全），通过后复制到本地库；
- 导出 `.zip`：只写入 `pet.json`（严格 UTF-8 无 BOM）与图集原始字节，即 Codex 上传格式；
- 引用（linked）的 Codex/UniPet 宠物不会被修改，删除操作只对本地库生效。

## 7. 校验报告

`bytepet validate <path>` 或界面里的「校验」会返回：

- `errors`：缺清单、图集缺失、解码失败、几何不匹配、路径穿越、无 idle 动画；
- `warnings`：几何推断、未使用但非透明的单元格。

## 8. 与 UniPet 的关系

UniPet 使用同一套 Codex 几何与 `pet.json` 文件名，因此两边宠物互通。本应用额外接受 UniPet 的 `frame`/`animations` 字段，也接受它的 `loopStart`、逐帧 `durationMs` 写法。本地协议的事件字段同样保持一致，便于社区 hook 直接复用。
