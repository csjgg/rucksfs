const pptxgen = require('pptxgenjs');

const pptx = new pptxgen();
pptx.layout = 'LAYOUT_WIDE';
pptx.author = '崔顺杰';
pptx.company = '华中科技大学';
pptx.subject = '毕业设计答辩';
pptx.title = '采用元数据键值对存储的文件操作设计与实现';
pptx.lang = 'zh-CN';
pptx.theme = {
  headFontFace: 'Noto Sans CJK SC',
  bodyFontFace: 'Noto Sans CJK SC',
  lang: 'zh-CN'
};
pptx.margin = 0;
pptx.layout = 'LAYOUT_WIDE';
pptx.defineLayout({ name: 'CUSTOM_WIDE', width: 13.333, height: 7.5 });
pptx.layout = 'CUSTOM_WIDE';
pptx.defineSlideMaster({
  title: 'RUCKSFS_MASTER',
  background: { color: 'F8F8F8' },
  objects: []
});

const C = {
  red: 'C41E3D',
  redDark: '8F1830',
  blue: '1A3A5C',
  blue2: '2E5E82',
  dark: '222222',
  gray: '666666',
  lightGray: 'E8EBEF',
  bg: 'F8F8F8',
  white: 'FFFFFF',
  green: '4E8F48',
  orange: 'D7862E'
};
const FONT = 'Noto Sans CJK SC';
const SLIDE_W = 13.333;
const SLIDE_H = 7.5;
const path = require('path');
const repoRoot = path.resolve(__dirname, '..');
const logo = path.join(repoRoot, 'scripts/assets/hust_logo.png');
const paths = {
  architecture: path.join(repoRoot, 'docs/thesis-template/images/architecture_v5_example.png'),
  key: path.join(repoRoot, 'docs/thesis-template/images/key_mapping.png'),
  delta: path.join(repoRoot, 'docs/thesis-template/images/delta_lazy_fold.png'),
  rename: path.join(repoRoot, 'docs/thesis-template/images/rename_lock_order.png'),
  openSeq: path.join(repoRoot, 'docs/thesis-template/images/open_sequence_diagram.png')
};
const dims = {
  [logo]: [3213, 646],
  [paths.architecture]: [1000, 560],
  [paths.key]: [2262, 884],
  [paths.delta]: [2070, 878],
  [paths.rename]: [1466, 500],
  [paths.openSeq]: [1256, 840]
};

function fitBox(path, x, y, w, h) {
  const [iw, ih] = dims[path];
  const ir = iw / ih;
  const br = w / h;
  if (ir > br) {
    const nh = w / ir;
    return { x, y: y + (h - nh) / 2, w, h: nh };
  }
  const nw = h * ir;
  return { x: x + (w - nw) / 2, y, w: nw, h };
}

function rect(slide, x, y, w, h, fill, line = fill, radius = false, transparency = 0) {
  slide.addShape(radius ? pptx.ShapeType.roundRect : pptx.ShapeType.rect, {
    x, y, w, h,
    fill: { color: fill, transparency },
    line: { color: line, transparency: line === fill ? 100 : 0, width: 0.6 },
    rectRadius: radius ? 0.08 : undefined
  });
}

function line(slide, x, y, w, h, color = C.lightGray, width = 1) {
  slide.addShape(pptx.ShapeType.line, { x, y, w, h, line: { color, width } });
}

function txt(slide, text, x, y, w, h, opts = {}) {
  slide.addText(text, {
    x, y, w, h,
    fontFace: FONT,
    color: opts.color || C.dark,
    fontSize: opts.size || 16,
    bold: !!opts.bold,
    italic: !!opts.italic,
    align: opts.align || 'left',
    valign: opts.valign || 'top',
    margin: opts.margin ?? 0.04,
    breakLine: opts.breakLine,
    fit: opts.fit,
    rotate: opts.rotate,
    paraSpaceAfterPt: opts.paraSpaceAfterPt ?? 0,
    breakLine: opts.breakLine
  });
}

function rich(slide, runs, x, y, w, h, opts = {}) {
  slide.addText(runs, {
    x, y, w, h,
    fontFace: FONT,
    color: opts.color || C.dark,
    fontSize: opts.size || 16,
    bold: !!opts.bold,
    align: opts.align || 'left',
    valign: opts.valign || 'top',
    margin: opts.margin ?? 0.04,
    breakLine: opts.breakLine,
    fit: opts.fit,
    paraSpaceAfterPt: opts.paraSpaceAfterPt ?? 0
  });
}

function header(slide, title, page) {
  rect(slide, 0, 0, SLIDE_W, 0.82, C.white, C.white);
  slide.addImage({ path: logo, ...fitBox(logo, 0.45, 0.18, 1.45, 0.30) });
  txt(slide, title, 2.15, 0.20, 8.9, 0.42, { size: 24, bold: true, color: C.dark, margin: 0 });
  rect(slide, 11.58, 0.21, 0.28, 0.28, C.red, C.red, true);
  txt(slide, String(page).padStart(2, '0'), 11.93, 0.18, 0.45, 0.26, { size: 10, bold: true, color: C.gray, margin: 0 });
  line(slide, 0.45, 0.81, 12.45, 0, C.lightGray, 0.7);
}

function footer(slide, page) {
  line(slide, 0.55, 7.08, 12.25, 0, 'D9DDE3', 0.5);
  txt(slide, 'RucksFS 毕业设计答辩', 0.62, 7.16, 2.8, 0.18, { size: 7.5, color: '8A8A8A', margin: 0 });
  txt(slide, `${page}/10`, 12.23, 7.16, 0.6, 0.18, { size: 7.5, color: '8A8A8A', align: 'right', margin: 0 });
}

function contentSlide(title, page) {
  const s = pptx.addSlide('RUCKSFS_MASTER');
  header(s, title, page);
  footer(s, page);
  return s;
}

function pill(slide, text, x, y, w, color = C.red, textColor = C.white) {
  rect(slide, x, y, w, 0.36, color, color, true);
  txt(slide, text, x, y + 0.07, w, 0.18, { size: 10.5, bold: true, color: textColor, align: 'center', margin: 0 });
}

function bulletBlock(slide, items, x, y, w, rowH = 0.48, opts = {}) {
  items.forEach((it, i) => {
    const yy = y + i * rowH;
    rect(slide, x, yy + 0.08, 0.16, 0.16, opts.dotColor || C.red, opts.dotColor || C.red, true);
    txt(slide, it, x + 0.30, yy, w - 0.3, rowH - 0.02, { size: opts.size || 15, color: opts.color || C.dark, margin: 0.01, fit: 'shrink' });
  });
}

function imageCard(slide, path, x, y, w, h, caption) {
  rect(slide, x, y, w, h, C.white, 'DDE2E8', true);
  const box = fitBox(path, x + 0.15, y + 0.15, w - 0.30, h - (caption ? 0.55 : 0.30));
  slide.addImage({ path, ...box });
  if (caption) txt(slide, caption, x + 0.18, y + h - 0.33, w - 0.36, 0.18, { size: 9, color: C.gray, align: 'center', margin: 0 });
}

function statCard(slide, value, label, x, y, w, h, color = C.red, sub = '') {
  rect(slide, x, y, w, h, C.white, 'DDE2E8', true);
  txt(slide, value, x + 0.16, y + 0.14, w - 0.32, 0.50, { size: 26, bold: true, color, align: 'center', margin: 0, fit: 'shrink' });
  txt(slide, label, x + 0.18, y + 0.72, w - 0.36, 0.30, { size: 11.5, bold: true, color: C.dark, align: 'center', margin: 0, fit: 'shrink' });
  if (sub) txt(slide, sub, x + 0.18, y + 1.08, w - 0.36, 0.20, { size: 8.5, color: C.gray, align: 'center', margin: 0, fit: 'shrink' });
}

function addBarGroup(slide, title, data, x, y, w, h, maxVal) {
  txt(slide, title, x, y - 0.35, w, 0.22, { size: 12.5, bold: true, color: C.dark, align: 'center', margin: 0 });
  const colors = [C.red, C.blue, C.orange];
  const labels = ['RucksFS', 'NFS', 'JuiceFS'];
  const gap = 0.08;
  const bw = (w - 0.20) / 3 - gap;
  data.forEach((v, i) => {
    const bh = Math.max(0.08, (v / maxVal) * h);
    const bx = x + 0.10 + i * (bw + gap);
    rect(slide, bx, y + h - bh, bw, bh, colors[i], colors[i]);
    txt(slide, String(v), bx - 0.05, y + h - bh - 0.20, bw + 0.10, 0.15, { size: 7.2, color: colors[i], bold: true, align: 'center', margin: 0, fit: 'shrink' });
    txt(slide, labels[i], bx - 0.10, y + h + 0.08, bw + 0.20, 0.15, { size: 6.5, color: C.gray, align: 'center', margin: 0, fit: 'shrink' });
  });
  line(slide, x, y + h, w, 0, 'BFC6CF', 0.6);
}

function plotLineChart(slide, x, y, w, h) {
  const np = [64, 96, 128, 192, 256, 384];
  const delta = [14726, 20574, 23042, 29602, 29734, 38081];
  const nod = [14773, 20479, 13179, 11703, 11985, 12855];
  const maxY = 40000;
  rect(slide, x, y, w, h, C.white, 'DDE2E8', true);
  const px = x + 0.65, py = y + 0.45, pw = w - 1.0, ph = h - 1.1;
  for (let i = 0; i <= 4; i++) {
    const gy = py + ph - (i / 4) * ph;
    line(slide, px, gy, pw, 0, 'E7EBF0', 0.45);
    txt(slide, String(i * 10000), x + 0.10, gy - 0.08, 0.42, 0.12, { size: 6.5, color: C.gray, align: 'right', margin: 0 });
  }
  line(slide, px, py + ph, pw, 0, 'AEB7C2', 0.8);
  line(slide, px, py, 0, ph, 'AEB7C2', 0.8);
  const xs = np.map((_, i) => px + (i / (np.length - 1)) * pw);
  const fy = v => py + ph - (v / maxY) * ph;
  function draw(vals, color) {
    for (let i = 0; i < vals.length - 1; i++) {
      line(slide, xs[i], fy(vals[i]), xs[i+1] - xs[i], fy(vals[i+1]) - fy(vals[i]), color, 2.2);
    }
    vals.forEach((v, i) => {
      rect(slide, xs[i] - 0.055, fy(v) - 0.055, 0.11, 0.11, color, color, true);
    });
  }
  draw(delta, C.red);
  draw(nod, C.blue);
  np.forEach((v, i) => txt(slide, String(v), xs[i]-0.18, py + ph + 0.12, 0.36, 0.12, { size: 6.8, color: C.gray, align: 'center', margin: 0 }));
  txt(slide, '并发 rank 数 np', px + pw/2 - 0.6, y + h - 0.28, 1.2, 0.14, { size: 7.5, color: C.gray, align: 'center', margin: 0 });
  txt(slide, 'Create 吞吐 (ops/s)', x + 0.12, y + 0.10, 1.1, 0.14, { size: 7.5, color: C.gray, margin: 0 });
  rect(slide, x + w - 1.85, y + 0.18, 0.15, 0.09, C.red, C.red);
  txt(slide, 'Delta', x + w - 1.63, y + 0.14, 0.5, 0.14, { size: 8.5, color: C.dark, margin: 0 });
  rect(slide, x + w - 1.05, y + 0.18, 0.15, 0.09, C.blue, C.blue);
  txt(slide, 'NoDelta', x + w - 0.83, y + 0.14, 0.72, 0.14, { size: 8.5, color: C.dark, margin: 0 });
}

// 1 Cover
{
  const s = pptx.addSlide('RUCKSFS_MASTER');
  s.background = { color: C.white };
  rect(s, 0, 0, SLIDE_W, 7.5, C.white, C.white);
  rect(s, 0, 0, 0.28, 7.5, C.red, C.red);
  rect(s, 0.28, 0, 0.08, 7.5, C.blue, C.blue);
  slideLogo = fitBox(logo, 4.80, 0.62, 3.75, 0.75);
  s.addImage({ path: logo, ...slideLogo });
  txt(s, '采用元数据键值对存储的文件操作\n设计与实现', 1.15, 2.02, 11.0, 1.18, { size: 34, bold: true, color: C.dark, align: 'center', margin: 0, fit: 'shrink' });
  rect(s, 4.92, 3.58, 3.50, 0.08, C.red, C.red);
  txt(s, '毕业设计答辩', 5.12, 3.82, 3.10, 0.30, { size: 17, bold: true, color: C.red, align: 'center', margin: 0 });
  txt(s, '学生：崔顺杰    学号：U202215522\n院系：计算机科学与技术学院    班级：计科2206\n指导教师：曹强    日期：2026年5月14日', 3.15, 5.45, 7.05, 0.72, { size: 14, color: C.dark, align: 'center', margin: 0.02, fit: 'shrink' });
  txt(s, '华中科技大学', 5.52, 6.47, 2.3, 0.22, { size: 12, color: C.gray, align: 'center', margin: 0 });
}

// 2 Background
{
  const s = contentSlide('背景与毕设目标', 2);
  txt(s, '问题背景', 0.72, 1.10, 2.0, 0.28, { size: 16, bold: true, color: C.red, margin: 0 });
  rect(s, 0.72, 1.52, 5.72, 4.60, C.white, 'DDE2E8', true);
  statCard(s, '60%–80%', '元数据操作占文件系统 I/O', 1.02, 1.85, 1.85, 1.38, C.red, 'stat / open / readdir 等');
  statCard(s, '≥6 次', '一次 ext4 create 随机 I/O', 3.05, 1.85, 1.85, 1.38, C.blue, '日志、inode、目录块等');
  statCard(s, '热点锁', '共享父目录串行化', 1.02, 3.55, 1.85, 1.38, C.orange, 'i_rwsem / 父 inode');
  rect(s, 3.25, 3.63, 2.80, 1.18, 'FFF3F3', 'F0C6C6', true);
  txt(s, '传统页式/B+树路径\n随机写 + 写放大 + 锁竞争', 3.45, 3.88, 2.40, 0.44, { size: 14, bold: true, color: C.redDark, align: 'center', margin: 0 });
  line(s, 6.72, 3.75, 0.75, 0, C.red, 2.0);
  s.addShape(pptx.ShapeType.chevron, { x: 7.18, y: 3.49, w: 0.45, h: 0.50, fill: { color: C.red }, line: { color: C.red } });
  txt(s, '设计目标', 8.05, 1.10, 2.0, 0.28, { size: 16, bold: true, color: C.red, margin: 0 });
  rect(s, 7.85, 1.52, 4.75, 4.60, C.white, 'DDE2E8', true);
  bulletBlock(s, [
    '保持标准 POSIX 语义，应用无需改动',
    'lookup 点查、readdir 前缀扫描、rename 常数步',
    '事务保证并发目录操作正确性',
    'DeltaOp 缓解共享父目录属性写热点'
  ], 8.20, 2.05, 4.05, 0.72, { size: 16 });
  pill(s, 'KV / LSM-tree 元数据路径', 8.45, 5.28, 3.50, C.blue);
}

// 3 Architecture
{
  const s = contentSlide('RucksFS 系统总体架构', 3);
  imageCard(s, paths.architecture, 0.76, 1.18, 7.65, 5.54, 'FUSE 客户端、MetadataServer、DataServer 三段式结构');
  statCard(s, 'Client', 'FUSE 请求入口', 8.72, 1.25, 1.33, 1.08, C.red, '路由 + 编排');
  statCard(s, 'MDS', '元数据核心', 10.20, 1.25, 1.33, 1.08, C.blue, '事务 + Delta');
  statCard(s, 'DS', '文件数据服务', 11.68, 1.25, 1.33, 1.08, C.green, '读写 + 截断');
  bulletBlock(s, [
    '客户端承接 Linux 内核 FUSE 请求',
    'MDS 管理 inode、目录项、事务和 DeltaOp',
    'DS 负责文件字节读写，当前为最小实现',
    '三者通过 gRPC 通信，职责边界清晰'
  ], 8.85, 2.85, 3.85, 0.62, { size: 15 });
  rect(s, 8.80, 5.85, 3.95, 0.54, 'FFF7F7', 'EFC7CF', true);
  txt(s, '重点：元数据路径，而非完整生产级容错系统', 9.02, 6.03, 3.55, 0.18, { size: 11.5, color: C.redDark, bold: true, align: 'center', margin: 0 });
}

// 4 Metadata model
{
  const s = contentSlide('元数据存储模型：目录树映射到 KV 键空间', 4);
  imageCard(s, paths.key, 0.70, 1.15, 8.05, 4.62, '路径树到 RocksDB 键空间的映射');
  rect(s, 0.94, 5.98, 7.55, 0.58, 'FFFFFF', 'DDE2E8', true);
  rich(s, [
    { text: '目录项键：', options: { bold: true, color: C.red } },
    { text: '[D][parent_inode_BE][child_name]  ' },
    { text: '大端序保证同一父目录前缀连续', options: { color: C.blue, bold: true } }
  ], 1.10, 6.18, 7.25, 0.18, { size: 13.5, margin: 0 });
  txt(s, '列族划分', 9.10, 1.15, 2.0, 0.25, { size: 16, bold: true, color: C.red, margin: 0 });
  const cfs = [
    ['inodes', 'inode 属性、数据位置、符号链接'],
    ['dir_entries', '目录项：父目录 → 子对象'],
    ['delta_entries', '父目录属性增量记录'],
    ['system', 'inode 分配器等低频状态']
  ];
  cfs.forEach((r, i) => {
    const yy = 1.64 + i * 0.80;
    rect(s, 9.05, yy, 3.65, 0.56, C.white, 'DDE2E8', true);
    txt(s, r[0], 9.22, yy + 0.13, 1.25, 0.13, { size: 9.7, bold: true, color: C.blue, margin: 0, fit: 'shrink' });
    txt(s, r[1], 10.58, yy + 0.11, 1.82, 0.16, { size: 9.4, color: C.dark, margin: 0, fit: 'shrink' });
  });
  pill(s, 'lookup 点查', 9.03, 5.30, 1.08, C.red);
  pill(s, 'readdir 前缀扫描', 10.25, 5.30, 1.46, C.blue);
  pill(s, 'rename 常数步', 9.68, 5.86, 1.42, C.green);
}

// 5 Delta
{
  const s = contentSlide('核心设计：DeltaOp 增量更新与懒折叠', 5);
  rect(s, 0.70, 1.10, 3.42, 5.62, C.white, 'DDE2E8', true);
  txt(s, '要解决的问题', 0.98, 1.42, 2.0, 0.25, { size: 16, bold: true, color: C.red, margin: 0 });
  bulletBlock(s, [
    'create / unlink 需要更新父目录 mtime、ctime、nlink',
    '朴素做法会读改写同一父 inode 键',
    '共享父目录高并发下触发行锁等待和事务重试'
  ], 1.02, 1.95, 2.70, 0.82, { size: 14 });
  rect(s, 1.02, 4.80, 2.55, 0.80, 'FFF3F3', 'EFC7CF', true);
  txt(s, '思路：覆盖写 → 追加增量', 1.15, 5.07, 2.30, 0.18, { size: 13, bold: true, color: C.redDark, align: 'center', margin: 0 });
  imageCard(s, paths.delta, 4.38, 1.10, 8.25, 5.62, '写路径追加 DeltaOp，读路径合并，后台按阈值懒折叠');
  pill(s, '阈值 32 条', 9.60, 5.92, 1.15, C.red);
  pill(s, 'InodeFoldedCache', 10.92, 5.92, 1.38, C.blue);
}

// 6 PCC
{
  const s = contentSlide('并发正确性：悲观事务与排序加锁', 6);
  imageCard(s, paths.rename, 0.76, 1.22, 7.58, 3.35, 'rename 事务按 inode 编号升序加锁避免死锁');
  rect(s, 0.88, 4.90, 7.34, 1.20, 'FFFFFF', 'DDE2E8', true);
  rich(s, [
    { text: 'TOCTOU：', options: { bold: true, color: C.red } },
    { text: '如果检查目标是否存在、目录是否为空发生在事务外，并发 create/rename 可能改变检查结果。' }
  ], 1.10, 5.16, 6.90, 0.22, { size: 13, margin: 0 });
  rich(s, [
    { text: '解决：', options: { bold: true, color: C.blue } },
    { text: '用 get_for_update 把检查和修改收进同一 RocksDB 悲观事务视图。' }
  ], 1.10, 5.62, 6.90, 0.22, { size: 13, margin: 0 });
  txt(s, '事务策略', 9.00, 1.22, 2.0, 0.25, { size: 16, bold: true, color: C.red, margin: 0 });
  bulletBlock(s, [
    'AtomicWriteBatch：跨列族原子提交',
    '按 inode 升序加锁：等待图无环',
    '冲突重试：50 μs 起始退避，最多 3 次',
    '失败边界清楚：最终映射为 EAGAIN'
  ], 9.05, 1.82, 3.55, 0.70, { size: 15 });
  statCard(s, 'rename', '最能体现事务设计', 9.25, 5.38, 1.60, 1.05, C.red, '多对象原子切换');
  statCard(s, 'PCC', '悲观并发控制', 11.02, 5.38, 1.55, 1.05, C.blue, '避免检查失效');
}

// 7 Implementation
{
  const s = contentSlide('系统实现与技术选择', 7);
  const layers = [
    ['FUSE 客户端', 'fuse3 + tokio', '承接内核请求，异步编排 MDS / DS'],
    ['RPC 通信', 'tonic / prost', 'gRPC 接口分层，多通道连接池'],
    ['MetadataServer', 'RocksDB TransactionDB', 'inode / dir_entries / DeltaOp / PCC 事务'],
    ['DataServer', 'RawDiskDataStore', '固定偏移映射，聚焦元数据路径验证'],
    ['语言与工程', 'Rust', '内存安全，适合并发基础设施原型']
  ];
  layers.forEach((r, i) => {
    const yy = 1.14 + i * 0.83;
    const color = i === 2 ? C.red : (i === 0 ? C.blue : 'FFFFFF');
    rect(s, 0.92, yy, 3.05, 0.54, color, color === 'FFFFFF' ? 'DDE2E8' : color, true);
    txt(s, r[0], 1.10, yy + 0.14, 2.65, 0.15, { size: 11.5, bold: true, color: color === 'FFFFFF' ? C.dark : C.white, align: 'center', margin: 0 });
    rect(s, 4.25, yy, 2.20, 0.54, C.white, 'DDE2E8', true);
    txt(s, r[1], 4.42, yy + 0.14, 1.86, 0.15, { size: 10.5, bold: true, color: C.blue, align: 'center', margin: 0, fit: 'shrink' });
    rect(s, 6.73, yy, 5.55, 0.54, C.white, 'DDE2E8', true);
    txt(s, r[2], 6.94, yy + 0.13, 5.12, 0.16, { size: 10.5, color: C.dark, align: 'center', margin: 0, fit: 'shrink' });
  });
  line(s, 2.45, 1.72, 0, 3.15, 'C6CCD5', 1.0);
  txt(s, '实现边界', 0.92, 5.75, 1.4, 0.22, { size: 14, bold: true, color: C.red, margin: 0 });
  rect(s, 2.05, 5.55, 10.22, 0.78, 'FFF8E8', 'F0D8A8', true);
  txt(s, '当前原型保持“单 MDS + 单默认 DS”主路径：重点验证元数据建模、事务控制、客户端协调与 DeltaOp 优化；副本容错、元数据分片和空间回收作为后续工作。', 2.30, 5.78, 9.72, 0.24, { size: 12.3, color: C.dark, align: 'center', margin: 0, fit: 'shrink' });
}

// 8 Correctness
{
  const s = contentSlide('正确性验证：pjdfstest', 8);
  statCard(s, '398', 'pjdfstest 总用例', 0.92, 1.32, 1.90, 1.38, C.blue, '13 类系统调用');
  statCard(s, '182/182', '实现范围内全部通过', 3.08, 1.32, 2.05, 1.38, C.red, '分布式部署下验证');
  statCard(s, '100%', '范围内通过率', 5.39, 1.32, 1.70, 1.38, C.green, '不是总用例伪装通过');
  rect(s, 0.92, 3.22, 6.17, 2.80, C.white, 'DDE2E8', true);
  txt(s, '范围划分', 1.16, 3.50, 1.40, 0.20, { size: 14, bold: true, color: C.red, margin: 0 });
  const rows = [
    ['实现范围内', '182', '本课题选择实现的接口'],
    ['mknod / mkfifo 相关', '167', '显式返回 EOPNOTSUPP'],
    ['单文件大小上限', '1', 'DataServer 最小实现边界'],
    ['框架跳过项', '48', 'utimensat / remount 等']
  ];
  rows.forEach((r, i) => {
    const yy = 3.92 + i * 0.42;
    txt(s, r[0], 1.18, yy, 2.0, 0.16, { size: 10.7, color: i === 0 ? C.red : C.dark, bold: i === 0, margin: 0 });
    txt(s, r[1], 3.65, yy, 0.55, 0.16, { size: 10.7, color: i === 0 ? C.red : C.dark, bold: true, align: 'right', margin: 0 });
    txt(s, r[2], 4.55, yy, 2.10, 0.16, { size: 10.2, color: C.gray, margin: 0 });
  });
  rect(s, 7.78, 1.32, 4.85, 4.70, C.white, 'DDE2E8', true);
  txt(s, '覆盖到的关键边界语义', 8.10, 1.72, 2.5, 0.20, { size: 14, bold: true, color: C.red, margin: 0 });
  bulletBlock(s, [
    '打开文件被 unlink 后延迟回收',
    'rename 覆盖多硬链接文件时只减少 nlink',
    '超长文件名错误码穿透 gRPC + FUSE',
    'rmdir 空目录检查纳入事务视图'
  ], 8.13, 2.25, 4.05, 0.62, { size: 14 });
  pill(s, '正确性是性能结果的前提', 8.63, 5.30, 3.20, C.blue);
}

// 9 Performance
{
  const s = contentSlide('性能评估：横向对比（N=32 hard 模式）', 9);
  rect(s, 0.78, 1.08, 3.55, 5.55, C.white, 'DDE2E8', true);
  txt(s, '测试设置', 1.06, 1.38, 1.3, 0.20, { size: 15, bold: true, color: C.red, margin: 0 });
  bulletBlock(s, [
    '1 台 64 核服务器 + 多台 2 核客户端',
    'mdtest hard：共享父目录元数据压力',
    '关闭客户端缓存，测量 MDS 真实路径',
    '对比：RucksFS、NFS v4.2、JuiceFS+TiKV'
  ], 1.08, 1.86, 2.85, 0.70, { size: 13.5 });
  rect(s, 1.08, 5.32, 2.75, 0.58, 'FFF3F3', 'EFC7CF', true);
  txt(s, '结论只针对元数据高并发写入场景', 1.22, 5.53, 2.45, 0.15, { size: 9.5, bold: true, color: C.redDark, align: 'center', margin: 0, fit: 'shrink' });
  rect(s, 4.70, 1.08, 7.82, 5.55, C.white, 'DDE2E8', true);
  addBarGroup(s, 'create', [8035, 1733, 4710], 5.05, 2.18, 1.85, 2.2, 25000);
  addBarGroup(s, 'stat', [10007, 24696, 8022], 7.42, 2.18, 1.85, 2.2, 25000);
  addBarGroup(s, 'remove', [8245, 5682, 3728], 9.78, 2.18, 1.85, 2.2, 25000);
  txt(s, '吞吐量单位：ops/s', 10.70, 1.38, 1.0, 0.16, { size: 8.5, color: C.gray, margin: 0 });
  rect(s, 5.12, 5.45, 2.20, 0.58, 'FFF3F3', 'EFC7CF', true);
  txt(s, 'create 相对 NFS：4.64×', 5.32, 5.66, 1.80, 0.15, { size: 10.8, bold: true, color: C.redDark, align: 'center', margin: 0 });
  rect(s, 7.58, 5.45, 3.70, 0.58, 'F1F6FB', 'CAD8E6', true);
  txt(s, 'stat：NFS 领先，承认 LSM 读路径代价', 7.78, 5.66, 3.30, 0.15, { size: 10.8, bold: true, color: C.blue, align: 'center', margin: 0 });
}

// 10 Delta result + summary
{
  const s = contentSlide('DeltaOp 对照实验与工作总结', 10);
  plotLineChart(s, 0.78, 1.08, 6.75, 4.58);
  statCard(s, '2.96×', 'np=384 create', 7.95, 1.16, 1.60, 1.18, C.red, '38081 vs 12855');
  statCard(s, '2.63×', 'np=384 remove', 9.75, 1.16, 1.60, 1.18, C.red, '32882 vs 12500');
  statCard(s, '≈1.00×', '低并发无明显差异', 11.55, 1.16, 1.35, 1.18, C.blue, 'np ≤ 96');
  rect(s, 7.95, 2.72, 4.95, 1.04, 'FFF8E8', 'F0D8A8', true);
  txt(s, '解释：DeltaOp 不是所有场景都提升；它针对的是共享父目录高并发下的父 inode 写锁争用。', 8.18, 3.02, 4.52, 0.28, { size: 11.8, color: C.dark, align: 'center', margin: 0, fit: 'shrink' });
  txt(s, '工作总结', 8.00, 4.28, 1.5, 0.20, { size: 15, bold: true, color: C.red, margin: 0 });
  bulletBlock(s, [
    '实现基于 RocksDB 多列族的 FUSE 文件系统原型',
    '完成边心键编码、DeltaOp、PCC 事务等核心路径',
    'pjdfstest 范围内全部通过，mdtest 验证高并发收益',
    '不足：DataServer 最小实现、缓存和 RPC 合并仍可改进'
  ], 8.02, 4.72, 4.70, 0.52, { size: 13.0 });
  rect(s, 0.96, 5.95, 6.20, 0.47, 'FFF3F3', 'EFC7CF', true);
  txt(s, '结论：KV 元数据模型 + DeltaOp 在共享父目录高并发元数据写入场景下具有可行性。', 1.15, 6.10, 5.82, 0.14, { size: 10.5, bold: true, color: C.redDark, align: 'center', margin: 0, fit: 'shrink' });
}

pptx.writeFile({ fileName: path.join(repoRoot, 'defense_presentation.pptx') });
