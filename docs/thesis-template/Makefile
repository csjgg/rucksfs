# Makefile for RucksFS thesis
#
# 常用命令：
#   make           编译（增量；首次或有改动时运行 biber + 多轮 xelatex）
#   make force     强制整编（清中间文件后完整跑一遍）
#   make view      在默认 PDF 阅读器中打开 main.pdf（macOS/Linux 自动检测）
#   make clean     清掉中间文件，但保留 main.pdf
#   make distclean 把 main.pdf 也删了

MAIN     := main
TEX      := $(MAIN).tex
PDF      := $(MAIN).pdf

LATEXMK  := latexmk
LATEXMK_FLAGS := -xelatex -interaction=nonstopmode -shell-escape -synctex=1

.PHONY: all force view clean distclean words help

all: $(PDF)

$(PDF): $(TEX) $(wildcard body/*.tex) $(wildcard template/*.cls template/*.sty) ref.bib
	$(LATEXMK) $(LATEXMK_FLAGS) $(TEX)

force: clean
	$(LATEXMK) $(LATEXMK_FLAGS) $(TEX)

view: $(PDF)
	@if command -v xdg-open >/dev/null 2>&1; then xdg-open $(PDF); \
	elif command -v open >/dev/null 2>&1; then open $(PDF); \
	else echo "未检测到 PDF 阅读器，请手动打开 $(PDF)"; fi

clean:
	$(LATEXMK) -c $(TEX)
	@rm -f $(MAIN).bbl $(MAIN).run.xml $(MAIN).synctex.gz $(MAIN).xdv \
	       $(MAIN).fdb_latexmk $(MAIN).fls

distclean:
	$(LATEXMK) -C $(TEX)
	@rm -f $(PDF) $(MAIN).bbl $(MAIN).run.xml $(MAIN).synctex.gz $(MAIN).xdv \
	       $(MAIN).fdb_latexmk $(MAIN).fls

# 统计 PDF 字符数（用于投稿字数限制检查）
words: $(PDF)
	@echo "含空格字符数：$$(pdftotext $(PDF) - | wc -m)"
	@echo "不含空格字符数：$$(pdftotext $(PDF) - | tr -d '[:space:]' | wc -m)"
	@echo "PDF 总页数：$$(pdfinfo $(PDF) | grep '^Pages:' | awk '{print $$2}')"

help:
	@echo "可用目标："
	@echo "  make           增量编译 main.pdf"
	@echo "  make force     清掉中间文件后完整重编"
	@echo "  make view      打开 main.pdf"
	@echo "  make clean     清理中间文件（保留 main.pdf）"
	@echo "  make distclean 清理所有输出（含 main.pdf）"
	@echo "  make words     统计 PDF 字符数"
