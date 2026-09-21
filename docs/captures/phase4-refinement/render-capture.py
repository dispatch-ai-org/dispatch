#!/usr/bin/env python3
"""Render recorded Dispatch VT output; this is not a native-terminal screenshot.

Uses Pillow. This deliberately accepts the VT subset emitted by these captures
and fails on unknown control instructions instead of inventing screen contents.
The supplied .ansi input is a complete byte prefix at a recorded milestone.
"""
import argparse
import json
import re
import unicodedata
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ANSI = ['#000000','#aa0000','#00aa00','#aa5500','#0000aa','#aa00aa','#00aaaa','#aaaaaa',
        '#555555','#ff5555','#55ff55','#ffff55','#5555ff','#ff55ff','#55ffff','#ffffff']


def color256(n):
    if n < 16: return ANSI[n]
    if n >= 232:
        v = 8 + 10 * (n-232)
        return f'#{v:02x}{v:02x}{v:02x}'
    n -= 16
    steps = [0, 95, 135, 175, 215, 255]
    return '#' + ''.join(f'{steps[v]:02x}' for v in [n//36, (n//6)%6, n%6])


class Screen:
    def __init__(self, width, height):
        self.width, self.height = width, height
        self.x = self.y = 0
        self.style = (None, None, False, False, False)
        self.rows = self.blank()
        self.saved = None
        self.cursor = True

    def blank(self):
        return [[(' ', (None, None, False, False, False)) for _ in range(self.width)]
                for _ in range(self.height)]

    def newline(self):
        self.y += 1
        if self.y == self.height:
            self.rows.pop(0)
            self.rows.append(self.blank()[0])
            self.y -= 1

    def char(self, c):
        if c == '\n':
            self.newline()
            return
        if c == '\r': self.x = 0; return
        if c == '\b': self.x = max(0, self.x-1); return
        if c == '\t': self.x = min(self.width-1, ((self.x//8)+1)*8); return
        if ord(c) < 32: return
        if unicodedata.combining(c):
            if self.x:
                old, style = self.rows[self.y][self.x-1]
                self.rows[self.y][self.x-1] = (old+c, style)
            return
        n = 2 if unicodedata.east_asian_width(c) in ('W','F') else 1
        if self.x >= self.width:
            self.x = 0
            self.newline()
        self.rows[self.y][self.x] = (c, self.style)
        if n == 2 and self.x+1 < self.width: self.rows[self.y][self.x+1] = ('', self.style)
        self.x += n

    def csi(self, params, command):
        private = params.startswith('?')
        p = [int(v or 0) for v in params.lstrip('?').split(';')]
        n = p[0] or 1
        if private and command in 'hl':
            for v in p:
                if v == 1049:
                    if command == 'h':
                        self.saved = (self.rows, self.x, self.y)
                        self.rows = self.blank()
                        self.x = self.y = 0
                    elif self.saved:
                        self.rows, self.x, self.y = self.saved
                        self.saved = None
                elif v == 25: self.cursor = command == 'h'
                elif v not in (2004, 1, 1000, 1002, 1003, 1006, 1004, 1015):
                    raise ValueError(f'Unsupported private mode {v}')
            return
        if command in 'Hf':
            self.y = min(self.height-1, (p[0] or 1)-1)
            self.x = min(self.width-1, ((p[1] if len(p)>1 else 1) or 1)-1)
        elif command == 'J':
            if p[0] in (2,3): self.rows = self.blank()
            elif p[0] == 0:
                self.rows[self.y][self.x:] = self.blank()[0][self.x:]
                for y in range(self.y+1, self.height): self.rows[y] = self.blank()[0]
            elif p[0] == 1:
                for y in range(self.y): self.rows[y] = self.blank()[0]
                self.rows[self.y][:self.x+1] = self.blank()[0][:self.x+1]
        elif command == 'K':
            lo, hi = ((0,self.width) if p[0]==2 else (0,self.x+1) if p[0]==1 else (self.x,self.width))
            self.rows[self.y][lo:hi] = self.blank()[0][lo:hi]
        elif command == 'm':
            fg,bg,bold,under,inverse = self.style
            i = 0
            while i < len(p):
                v = p[i]
                if v == 0: fg,bg,bold,under,inverse = None,None,False,False,False
                elif v in (1,22): bold = v == 1
                elif v in (4,24): under = v == 4
                elif v in (7,27): inverse = v == 7
                elif v == 39: fg = None
                elif v == 49: bg = None
                elif 30 <= v <= 37: fg = ANSI[v-30]
                elif 90 <= v <= 97: fg = ANSI[v-90+8]
                elif 40 <= v <= 47: bg = ANSI[v-40]
                elif v in (38,48):
                    if p[i+1] == 5: col = color256(p[i+2]); i += 2
                    elif p[i+1] == 2: col = '#'+''.join(f'{x:02x}' for x in p[i+2:i+5]); i += 4
                    else: raise ValueError(f'Unsupported extended color {p}')
                    if v == 38: fg = col
                    else: bg = col
                elif v != 59: raise ValueError(f'Unsupported style {v}')  # 59: default underline color
                i += 1
            self.style = fg,bg,bold,under,inverse
        elif command == 'n': pass  # A query, not visible terminal content.
        elif command == 'A': self.y = max(0,self.y-n)
        elif command == 'B': self.y = min(self.height-1,self.y+n)
        elif command == 'C': self.x = min(self.width-1,self.x+n)
        elif command == 'D': self.x = max(0,self.x-n)
        else: raise ValueError(f'Unsupported CSI {params}{command}')

    def feed(self, text):
        position = 0
        for match in re.finditer(r'\x1b\[([0-?]*)([ -/]*)([@-~])', text):
            for c in text[position:match.start()]:
                if c == '\x1b': raise ValueError('Unsupported escape sequence')
                self.char(c)
            if match[2]: raise ValueError('Unsupported CSI intermediate')
            self.csi(match[1], match[3])
            position = match.end()
        for c in text[position:]: self.char(c)


def main():
    args = argparse.ArgumentParser()
    args.add_argument('input', type=Path)
    args.add_argument('output', type=Path)
    args.add_argument('--width', type=int, default=100)
    args.add_argument('--height', type=int, default=30)
    args.add_argument('--font', default='/System/Library/Fonts/Menlo.ttc')
    args.add_argument('--background', default='#070B10')
    args.add_argument('--foreground', default='#F8FAFC')
    options = args.parse_args()
    screen = Screen(options.width, options.height)
    screen.feed(options.input.read_bytes().decode('utf-8'))
    used = [i for i,row in enumerate(screen.rows) if any(c.strip() for c,_ in row)]
    first,last = (min(used),max(used)+1) if used else (0,1)
    rows = screen.rows[first:last]
    font = ImageFont.truetype(options.font, 18)
    bold = ImageFont.truetype(options.font, 18, index=1)
    cell, line, pad = round(font.getlength('M')), 27, 20
    image = Image.new('RGB',(options.width*cell+pad*2,len(rows)*line+pad*2),options.background)
    draw = ImageDraw.Draw(image)
    for y,row in enumerate(rows):
        for x,(c,style) in enumerate(row):
            fg,bg,isbold,under,inverse = style
            fg,bg = fg or options.foreground, bg or options.background
            if inverse: fg,bg = bg,fg
            px,py = pad+x*cell,pad+y*line
            if bg != options.background: draw.rectangle((px,py,px+cell,py+line),fill=bg)
            draw.text((px,py),c,font=bold if isbold else font,fill=fg)
            if under: draw.line((px,py+line-3,px+cell,py+line-3),fill=fg,width=1)
    options.output.parent.mkdir(parents=True,exist_ok=True)
    image.save(options.output)
    options.output.with_suffix('.txt').write_text('\n'.join(''.join(c for c,_ in row).rstrip() for row in rows)+'\n')
    options.output.with_suffix('.render.json').write_text(json.dumps({
        'source': options.input.name, 'terminal_columns': options.width,
        'terminal_rows': options.height, 'cropped_blank_rows_before': first,
        'cropped_blank_rows_after': options.height-last,
        'background_for_preview': options.background, 'native_terminal_screenshot': False,
        'font': Path(options.font).name},indent=2)+'\n')

if __name__ == '__main__': main()
