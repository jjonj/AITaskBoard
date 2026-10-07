from PIL import Image, ImageDraw, ImageFilter

S=4
W=1024*S
img=Image.new("RGBA",(W,W),(0,0,0,0))
d=ImageDraw.Draw(img)

def rr(box,r,fill,outline=None,width=1):
    box=tuple(int(v*S) for v in box)
    d.rounded_rectangle(box,radius=int(r*S),fill=fill,outline=outline,width=int(width*S))

def rect(box,fill):
    d.rectangle(tuple(int(v*S) for v in box),fill=fill)

# shadow/glow
shadow=Image.new("RGBA",(W,W),(0,0,0,0))
sd=ImageDraw.Draw(shadow)
sd.rounded_rectangle((78*S,82*S,946*S,950*S),radius=190*S,fill=(0,0,0,165))
shadow=shadow.filter(ImageFilter.GaussianBlur(26*S))
img.alpha_composite(shadow)

glow=Image.new("RGBA",(W,W),(0,0,0,0))
gd=ImageDraw.Draw(glow)
gd.rounded_rectangle((70*S,70*S,954*S,954*S),radius=194*S,outline=(230,157,64,155),width=22*S)
glow=glow.filter(ImageFilter.GaussianBlur(18*S))
img.alpha_composite(glow)

# frame/background
rr((74,74,950,950),190,(205,132,48,255))
rr((92,92,932,932),174,(244,186,90,255))
rr((108,108,916,916),160,(17,23,32,255), (55,66,84,255), 5)

# columns
cols=[(172,226,360,790,(58,131,236,255),(35,63,96,255)),
      (418,208,606,790,(221,165,52,255),(56,57,74,255)),
      (664,226,852,790,(61,167,124,255),(36,72,63,255))]
for x1,y1,x2,y2,hc,bc in cols:
    rr((x1,y1,x2,y2),38,bc,(88,101,121,210),4)
    rr((x1,y1,x2,y1+98),38,hc)
    rect((x1,y1+62,x2,y1+98),hc)

# cards
def card(x,y,w=136,h=110,dot=(100,150,220,255)):
    rr((x,y,x+w,y+h),23,(236,240,246,255),(185,193,204,255),5)
    rr((x+24,y+29,x+92,y+44),7,(132,143,158,255))
    rr((x+24,y+60,x+108,y+75),7,(151,160,173,255))
    d.ellipse(((x+w-35)*S,(y+21)*S,(x+w-14)*S,(y+42)*S),fill=dot)

card(198,392,dot=(63,142,255,255))
card(198,535,dot=(63,142,255,255))
card(690,392,dot=(55,184,132,255))
card(690,535,dot=(55,184,132,255))

# dashed drop zone
for y in range(545,676,32):
    rr((448,y,574,min(y+16,690)),5,(103,115,132,180))
for x in range(448,575,32):
    rr((x,674,min(x+16,590),686),5,(103,115,132,180))

# floating card glow
fg=Image.new("RGBA",(W,W),(0,0,0,0))
fgd=ImageDraw.Draw(fg)
fgd.rounded_rectangle((392*S,410*S,636*S,560*S),radius=30*S,fill=(255,184,62,210))
fg=fg.filter(ImageFilter.GaussianBlur(22*S))
img.alpha_composite(fg)
d=ImageDraw.Draw(img)
rr((392,410,636,560),30,(255,247,226,255),(255,193,73,255),8)
rr((428,449,536,466),8,(133,143,157,255))
rr((428,483,562,500),8,(157,165,176,255))
d.ellipse((584*S,439*S,614*S,469*S),fill=(239,168,42,255))

# cursor arrow
pts=[(610,542),(610,684),(649,646),(692,714),(728,692),(686,626),(741,623)]
pts=[(x*S,y*S) for x,y in pts]
d.polygon(pts,fill=(247,249,252,255))
d.line(pts+[pts[0]],fill=(27,33,43,255),width=11*S,joint="curve")

img=img.resize((1024,1024),Image.Resampling.LANCZOS)
img.save(r"D:\SSDProjects\Tools\TaskBoard\src-tauri\icons\taskboard-source.png")
print("saved")
