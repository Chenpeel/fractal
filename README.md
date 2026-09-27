# fractal

用进制 $b$ 和方向数 $\text{dir}$ 绘制分形, 在交互窗口中实时调参和缩放.

## 示例动画

> 以下 GIF 均为 720x540. 示例命令将新动画写入 `out/`; `anim` 的默认输出尺寸为 1600x1200.

### 龙形曲线

![图: `linear` 规则, $(b,\text{dir})=(2,4)$, 龙形曲线的绘制过程.](assets/dragon_2_4.gif)

```bash
cargo run --release -- anim --base 2 --dir 4 --rule linear --steps 32768 --story draw --fps 12 --secs 4 --gif --width 720 --height 540 --out out/dragon_2_4.gif
```

### 科赫弧

![图: $(b,\text{dir})=(4,6)$ 和 `fold` 规则, GUI 无参数启动时的默认形状.](assets/koch_4_6_fold.gif)

```bash
cargo run --release -- anim --base 4 --dir 6 --rule fold --steps 65536 --story draw --fps 15 --secs 5 --gif --width 720 --height 540 --out out/koch_4_6_fold.gif
```

### 较密的轨迹

![图: $(b,\text{dir})=(3,7)$, 用 `linear` 规则观察较密的曲线结构.](assets/fill_3_7.gif)


```bash
cargo run --release -- anim --base 3 --dir 7 --rule linear --steps 177147 --story draw --fps 15 --secs 5 --gif --width 720 --height 540 --out out/fill_3_7.gif
```

### 深渲染

![图: $2^{50}$ 步无枚举, `grow` 动画按自相似层级揭示图形.](assets/deep_2_4_2p50.gif)


```bash
cargo run --release -- anim --base 2 --dir 4 --rule linear --steps 2^50 --story grow --fps 15 --secs 5 --gif --width 720 --height 540 --out out/deep_2_4_2p50.gif
```

### 亮色主题

![图: 在相同的龙形曲线参数下, 使用 `--bg light` 切换导出背景.](assets/dragon_2_4_light.gif)


```bash
cargo run --release -- anim --base 2 --dir 4 --rule linear --steps 32768 --bg light --story draw --fps 12 --secs 4 --gif --width 720 --height 540 --out out/dragon_2_4_light.gif
```

### 高进制曲线

![图: $(b,\text{dir})=(5,8)$, 观察高进制参数产生的轨迹.](assets/fill_5_8.gif)



```bash
cargo run --release -- anim --base 5 --dir 8 --rule linear --steps 390625 --story draw --fps 15 --secs 5 --gif --width 720 --height 540 --out out/fill_5_8.gif
```

### 收缩规则

![图: $(b,\text{dir})=(3,5)$ 的 `alternate` 规则满足 $\lvert S\rvert\approx0.618<1$.](assets/alt_3_5.gif)


```bash
cargo run --release -- anim --base 3 --dir 5 --rule alternate --steps 59049 --story draw --fps 15 --secs 5 --gif --width 720 --height 540 --ss 1 --out out/alt_3_5.gif
```

### 稀疏骨架

![图: $(b,\text{dir})=(6,4)$ 的 `linear` 轨迹呈现稀疏结构.](assets/skel_6_4.gif)



```bash
cargo run --release -- anim --base 6 --dir 4 --rule linear --steps 200000 --story grow --fps 15 --secs 5 --gif --width 720 --height 540 --out out/skel_6_4.gif
```

### 镜像

![图: 将方向数设为 $-7$, 对比前面 $(3,7)$ 的轨迹.](assets/mirror_3_-7.gif)



```bash
cargo run --release -- anim --base 3 --dir -7 --rule linear --steps 177147 --story grow --fps 15 --secs 5 --gif --width 720 --height 540 --out out/mirror_3_-7.gif
```

### 巡游缩放

![图: `tour` 故事先绘制曲线, 再逐级放大观察自相似结构.](assets/tour_3_7_light.gif)


```bash
cargo run --release -- anim --base 3 --dir 7 --rule linear --steps 177147 --story tour --bg light --fps 15 --secs 6 --gif --width 720 --height 540 --out out/tour_3_7_light.gif
```

## 数学原理

设 $n$ 的 $b$ 进制各位数字为 $r_i$, 逐位权重为 $g(r)$, 定义 $f(n)=\sum_i g(r_i)$. 第 $n$ 步的方向和位置满足:

$$\theta_n=\frac{2\pi}{\text{dir}}f(n), \qquad z_{n+1}=z_n+e^{i\theta_n}.$$

`--rule` 决定 $g$, 其中 $0\le r<b$:

| 规则 | $g(r)$ | 说明 |
|---|---|---|
| `linear` | $r$ | $f(n)=s_b(n)$, 即 $n$ 的 $b$ 进制数字和 |
| `alternate` | $(-1)^r r$ | 符号随数字值交替 |
| `fold` | $(-1)^r\min(r,\,b-1-r)$ | 先折到半程再交替符号; $(b,\text{dir})=(4,6)$ 给出科赫弧 |

$\text{dir}$ 表示把整圆分成多少份: $f(n)$ 每增加 1 单位, 朝向变化 $360^\circ/\text{dir}$. 负方向数产生镜像. 朝向只由 $n$ 决定, 不累加前一步朝向; 每步长度为 1.

令 $\zeta=e^{2\pi i/\text{dir}}$. 对三种规则均有 $f(bm+r)=f(m)+g(r)$, 因此得到块恒等式及自相似因子:

$$z(mb^L+j)=S^Lz(m)+\zeta^{f(m)}z(j), \qquad 0\le j<b^L, \qquad S=\sum_{r=0}^{b-1}\zeta^{g(r)}.$$

`linear` 规则下 $g(r)=r$, 等比求和给出:

$$S=\begin{cases}\dfrac{1-\zeta^b}{1-\zeta},&\zeta\ne1,\\[4pt]b,&\zeta=1.\end{cases}$$

设 $d=\lvert\text{dir}\rvert$. 当 $d>1$ 时, `linear` 规则满足 $\lvert S\rvert=\lvert\sin(\pi b/d)\rvert/\sin(\pi/d)$. 令 $r=b\bmod d$:

| 条件 | 结果 |
|---|---|
| $r=0$ | $S=0$: 每 $b^k$ 步回到原点, 轨迹有界, 不同顶点数不超过 $bd+1$ |
| $r\in\{1,d-1\}$ | $\lvert S\rvert=1$: 有界周期轨迹 |
| 其他余数 | $\lvert S\rvert>1$: 轨迹按层级扩张, 但不一定填满平面 |

$d=1$ 时 $\zeta=1$, 所有规则的每一步都沿同一方向, $S=b$, 轨迹是直线; 不能套用上表的 $r=0$ 情况. `fold` 在 $b=2$ 时所有权重均为 0, 同样退化成直线, 即使 $\lvert S\rvert=2>1$ 也不是分形. `alternate` 的 $(b,d)=(3,5)$ 则有 $\lvert S\rvert\approx0.618<1$, 产生有界但非周期的轨迹.

科赫弧取 $(b,\text{dir})=(4,6)$ 和 `fold`: $S=1+\zeta^{-1}+\zeta+1=3$. 每级分为 4 份, 缩放比为 3, 理论维数为 $\log 4/\log 3\approx1.262$. 相同参数的 `linear` 规则只有 $\lvert S\rvert=\sqrt{3}$, 不能得到这一科赫弧. $(b,\text{dir})=(2,4)$ 的 `linear` 规则则有 $S=1+i$, 对应龙形曲线的复基 $1+i$.

轨迹疏密还取决于角度和重叠, 不能仅凭 $d>b$ 或 $d\le b$ 判定. 例如 $(2,12)$ 的 `linear` 规则虽然 $d>b$ 且 $\lvert S\rvert>1$, 相似维数上界仅约 $1.053$, 不是面积填充; $(6,3)$ 则有 $d\le b$ 且 $S=0$, 并不扩张. 上方 $(3,7)$ 和 $(6,4)$ 的动画只是不同疏密形态的具体示例.

## 运行

```bash
cargo run --release                                   # 打开 GUI (默认 b=4 dir=6 rule=fold steps=65536)
cargo run --release -- render --base 3 --dir 6 --steps 177147 --out out/hex60
cargo run --release -- render --base 2 --dir 4 --steps 1125899906842624 --zoom 1e4 --out out/deep
cargo run --release -- anim   --base 2 --dir 5 --steps 200000 --story grow --secs 6 --out out/pentagon
cargo run --release -- anim   --base 3 --dir 3 --steps 59049 --gif --secs 5 --out out/closed
cargo run --release -- --help
cargo test                                            # 运行测试
```

`render` 输出 PNG (小步数另有 SVG); `anim` 默认把 RGB 帧直接管道给 `ffmpeg` 生成 H.264 MP4 (无中间帧落盘), 没有 `ffmpeg` 时用 `--gif` (内置零依赖 GIF89a 编码器) 或 `--frames-dir` 输出 PNG 序列.

### 全部参数

`适用` 一列标明参数对哪个子命令生效 (`全部` = gui / render / anim). `render` 和 `anim` 的默认进制, 方向数, 规则是 $(2,4,\text{linear})$; GUI 无参数启动时使用 $(4,6,\text{fold})$. 在 `gui` 命令中显式指定上述参数或 `--steps` 时, 未指定的参数沿用 `render` / `anim` 默认值.

| 参数 | 含义 | 适用 | 默认 |
|---|---|---|---|
| `--base B` | 进制 2..64 | 全部 | 2 (GUI 无参数: 4) |
| `--dir D` | 圆周等分数 $\lvert D\rvert$ 1..64, 负号=镜像 | 全部 | 4 (GUI 无参数: 6) |
| `--rule R` | 逐位转角表 `linear` / `alternate` / `fold` | 全部 | linear (GUI 无参数: fold) |
| `--steps N` | 步数 1..$2^{50}$, 支持 `2^30` / `1.5e6` / `2_000_000` | 全部 | 65536 |
| `--bg T` | 主题 `dark` / `light` | 全部 | dark |
| `--lang L` | 界面语言 `en` / `zh` | gui | en |
| `--width W` `--height H` | 输出尺寸 (render / anim) 或窗口尺寸 (gui) | 全部 | render/anim 1600x1200, gui 1500x900 |
| `--ss S` | 超采样 1..4 | render/anim | 2 |
| `--hue H` | 色相偏移 0..1 | 全部 | 0 |
| `--gain G` | 描边浓度 (0,10] | render/anim | 1 |
| `--zoom Z` `--center X,Y` | 放大倍数与视口中心 (世界坐标) | render | 1, 图形中心 |
| `--out PATH` | 输出路径前缀 (自动去扩展名) | render/anim | `out/curve_b{B}_d{D}_n{N}` |
| `--story S` `--fps F` `--secs T` | story `draw` / `grow` / `tour`, 帧率, 时长 | anim | draw, 30, 8 |
| `--gif` | 输出 GIF 而不是 MP4 | anim | 关 |
| `--frames-dir D` | 输出 PNG 帧序列到目录 D | anim | 关 |
| `--no-svg` | 不写 SVG | render | 关 |
| `--quiet` | 关闭动画逐帧进度输出 | anim | 关 |
| `--frames N` | 渲染 N 帧后自动退出 (冒烟测试) | gui | 关 |
| `--shot P` | 退出前把整个窗口截图为 P | gui | 关 |

