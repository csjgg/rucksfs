$pdf_mode = 5;              # 使用 xelatex
$xelatex = 'xelatex -interaction=nonstopmode -shell-escape -synctex=1 %O %S';
$bibtex_use = 2;            # 总是运行 biber
$biber = 'biber %O %B';

# 依赖追踪：图片和引用文件变化时重新编译
@default_files = ('main.tex');

# 清理时要删掉的中间文件
$clean_ext = 'bbl run.xml synctex.gz fdb_latexmk fls xdv';
