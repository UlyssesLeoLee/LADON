from pathlib import Path
from html import escape
import zipfile, os, textwrap, math
from PIL import Image, ImageDraw, ImageFont
import cairosvg

OUT=Path('/mnt/data/LADON_Explorer_IPA_V1_2_Full/ui-modern-svg')
OUT.mkdir(exist_ok=True)
W,H=1600,1000
C={'bg':'#EDEFEA','window':'#FFFFFF','sidebar':'#F8F9F6','main':'#FFFFFF','top':'#FBFCFA','ink':'#27312C','muted':'#738078','subtle':'#9CA69E','line':'#E5EAE4','light':'#F3F6F1','accent':'#718570','accentDark':'#4E684F','accentLight':'#EAF0E7','gold':'#B6A58E','softGold':'#F5F1EB','selected':'#EDF3E9','risk':'#B16E63','riskLight':'#F9EFEB','blue':'#637D8E','blueLight':'#EAF0F5'}
FONT="'Noto Sans CJK SC','Microsoft YaHei','Yu Gothic UI',sans-serif"
FMONO="'Cascadia Code','Noto Sans Mono CJK SC',monospace"

SYMS={
'folder':'<path d="M3 6.5a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
'file':'<path d="M6 3h7l5 5v13H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M13 3v5h5"/><path d="M8 13h7M8 17h6"/>',
'chevron-right':'<path d="m9 6 6 6-6 6"/>','chevron-down':'<path d="m6 9 6 6 6-6"/>','chevron-up':'<path d="m6 15 6-6 6 6"/>',
'arrow-left':'<path d="m14 6-6 6 6 6"/><path d="M8 12h12"/>','arrow-right':'<path d="m10 6 6 6-6 6"/><path d="M4 12h12"/>','arrow-up':'<path d="m6 14 6-6 6 6"/><path d="M12 8v12"/>',
'search':'<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
'more':'<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
'home':'<path d="m3 11 9-7 9 7"/><path d="M5 10v10h14V10"/><path d="M10 20v-6h4v6"/>',
'clock':'<circle cx="12" cy="12" r="9"/><path d="M12 7v5l4 2"/>',
'check':'<path d="m4 12 5 5L20 6"/>',
'circle-check':'<circle cx="12" cy="12" r="9"/><path d="m7 12 3.5 3.5 6.5-7"/>',
'cloud':'<path d="M7 18h11a4 4 0 0 0 .2-8A7 7 0 0 0 5 9a4.5 4.5 0 0 0 2 9z"/>',
'cloud-upload':'<path d="M7 18h11a4 4 0 0 0 0-8 7 7 0 0 0-13-1 4.5 4.5 0 0 0 2 9"/><path d="m9 15 3-3 3 3M12 12v9"/>',
'bot':'<rect x="4" y="7" width="16" height="13" rx="4"/><path d="M12 3v4M9 3h6"/><circle cx="9" cy="13" r="1"/><circle cx="15" cy="13" r="1"/><path d="M9 17h6"/>',
'users':'<circle cx="9" cy="8" r="3"/><path d="M3 19v-2a6 6 0 0 1 12 0v2"/><path d="M17 5a3 3 0 0 1 0 6M17 15a5 5 0 0 1 4 4"/>',
'message':'<path d="M5 5h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H9l-5 3v-5a2 2 0 0 1-1-2V7a2 2 0 0 1 2-2z"/>',
'layout':'<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>',
'git':'<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="12" r="2"/><path d="M6 7v10M6 9c0 5 12-2 12 3"/>',
'shield':'<path d="m12 2 8 4v6c0 5-3 8-8 10-5-2-8-5-8-10V6z"/><path d="m8 12 3 3 5-6"/>',
'settings':'<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2"/>',
'tag':'<path d="M3 4h9l9 9-8 8-10-10z"/><circle cx="8" cy="9" r="1"/>',
'paperclip':'<path d="m20 12-9 9a6 6 0 0 1-8-8l9-9a4 4 0 0 1 6 6l-9 9a2 2 0 0 1-3-3l9-9"/>',
'send':'<path d="m22 2-9 20-3-8-8-3z"/><path d="M22 2 10 14"/>',
'plus':'<path d="M12 4v16M4 12h16"/>',
'copy':'<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>',
'pencil':'<path d="m4 20 5-1 11-11-4-4L5 15z"/><path d="m14 6 4 4"/>',
'archive':'<rect x="3" y="4" width="18" height="5" rx="1"/><rect x="5" y="9" width="14" height="12" rx="1"/><path d="M10 13h4"/>',
'trash':'<path d="M4 7h16M9 7V4h6v3M6 7l1 14h10l1-14M10 11v6M14 11v6"/>',
'lock':'<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
'info':'<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7h.01"/>',
'sparkles':'<path d="m12 2 1.5 6.5L20 10l-6.5 1.5L12 18l-1.5-6.5L4 10l6.5-1.5zM20 18l.7 2.3L23 21l-2.3.7L20 24l-.7-2.3L17 21l2.3-.7z"/>',
'history':'<path d="M3 12a9 9 0 1 0 3-7M3 3v6h6"/><path d="M12 7v5l4 2"/>',
'share':'<path d="M12 16V3m0 0L7 8m5-5 5 5"/><path d="M4 14v5a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5"/>',
'columns':'<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M12 4v16"/>',
'eye':'<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>',
'external':'<path d="M13 4h7v7M20 4l-10 10"/><path d="M20 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h5"/>',
'alert':'<path d="m12 3 10 18H2z"/><path d="M12 9v5M12 17h.01"/>',
'check-square':'<rect x="3" y="3" width="18" height="18" rx="4"/><path d="m7 12 3 3 7-7"/>',
'download':'<path d="M12 3v13m0 0 5-5m-5 5-5-5M4 17v4h16v-4"/>',
'hard-drive':'<rect x="3" y="5" width="18" height="14" rx="3"/><path d="M3 12h18M16 16h.01M19 16h.01"/>',
'list':'<path d="M9 6h12M9 12h12M9 18h12"/><path d="M4 6h.01M4 12h.01M4 18h.01"/>',
'filter':'<path d="M3 4h18l-7 8v8l-4-2v-6z"/>',
'link':'<path d="m10 13 4-4M8 16H6a4 4 0 0 1 0-8h4M14 8h4a4 4 0 0 1 0 8h-4"/>',
'chevrons':'<path d="m7 7 5 5-5 5m6-10 5 5-5 5"/>',
'calendar':'<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M7 3v4M17 3v4M3 10h18"/>',
'key':'<circle cx="8" cy="13" r="5"/><path d="m12 10 9-7v5h-4v4h-4"/>',
'refresh':'<path d="M20 6v5h-5M4 18v-5h5"/><path d="M5 10a8 8 0 0 1 14-3l1 4M4 13l1 4a8 8 0 0 0 14-3"/>',
'grid':'<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
'ellipsis':'<circle cx="12" cy="5" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="12" cy="19" r="1"/>',
'sliders':'<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3"/><circle cx="16" cy="17" r="3"/>',
}

class SVG:
 def __init__(self,title):
  self.a=[f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="{escape(title)}">']
  self.a.append('<defs><filter id="shadow" x="-20%" y="-20%" width="140%" height="160%"><feDropShadow dx="0" dy="12" stdDeviation="22" flood-color="#26362A" flood-opacity=".08"/></filter><filter id="popover" x="-40%" y="-40%" width="180%" height="200%"><feDropShadow dx="0" dy="12" stdDeviation="15" flood-color="#26362A" flood-opacity=".13"/></filter>')
  for name,body in SYMS.items():
   self.a.append(f'<symbol id="i-{name}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{body}</symbol>')
  self.a.append('</defs>')
 def raw(self,s):self.a.append(s)
 def r(self,x,y,w,h,fill='none',stroke=None,rad=0,sw=1,opacity=None,shadow=None):
  t=f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rad}" fill="{fill}"'
  if stroke:t+=f' stroke="{stroke}" stroke-width="{sw}"'
  if opacity is not None:t+=f' opacity="{opacity}"'
  if shadow:t+=f' filter="url(#{shadow})"'
  self.a.append(t+'/>')
 def tx(self,x,y,s,size=16,color=None,weight=400,anchor='start',family=None,opacity=None,spacing=None):
  color=color or C['ink'];family=family or FONT
  t=f'<text x="{x}" y="{y}" fill="{color}" font-family="{family}" font-size="{size}" font-weight="{weight}" text-anchor="{anchor}" dominant-baseline="middle"'
  if opacity:t+=f' opacity="{opacity}"'
  if spacing:t+=f' letter-spacing="{spacing}"'
  self.a.append(t+'>'+escape(str(s))+'</text>')
 def l(self,x1,y1,x2,y2,color=None,sw=1,opacity=None):
  x=f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{color or C["line"]}" stroke-width="{sw}"'
  if opacity:x+=f' opacity="{opacity}"'
  self.a.append(x+'/>')
 def ic(self,name,x,y,size=20,col=None,opacity=1):
  self.a.append(f'<use href="#i-{name}" x="{x}" y="{y}" width="{size}" height="{size}" color="{col or C["muted"]}" opacity="{opacity}"/>')
 def circ(self,cx,cy,rad,fill='none',stroke=None,sw=1):
  self.a.append(f'<circle cx="{cx}" cy="{cy}" r="{rad}" fill="{fill}"'+(f' stroke="{stroke}" stroke-width="{sw}"' if stroke else '')+'/>')
 def pill(self,x,y,t,bg=None,col=None,fontsize=12,w=None,border=None):
  w=w or (len(t)*fontsize*.95 + 24)
  self.r(x,y,w,28,bg or C['light'],border,10)
  self.tx(x+w/2,y+14,t,fontsize,col or C['muted'],500,'middle');return w
 def btn(self,x,y,text,icon=None,w=None,primary=False):
  w=w or max(110,23+len(text)*17+(16 if icon else 0))
  self.r(x,y,w,42,C['accentDark'] if primary else '#FFFFFF',None if primary else C['line'],12)
  if icon:self.ic(icon,x+14,y+11,18,'#FFFFFF' if primary else C['muted']);self.tx(x+40,y+22,text,14,'#FFFFFF' if primary else C['ink'],500)
  else:self.tx(x+w/2,y+22,text,14,'#FFFFFF' if primary else C['ink'],500,'middle')
 def finish(self):self.a.append('</svg>');return '\n'.join(self.a)

def base(s,active='浏览',selected='项目设计',context=None,detail=False,chat='slim',quiet=False):
 s.r(0,0,W,H,C['bg'])
 # soft modern stacked circular ornament only a tiny abstract motif
 s.circ(1517,30,280,'#E7ECE6')
 s.r(54,42,1492,910,C['window'],None,22,shadow='shadow')
 s.r(55,43,1490,61,C['top'],None,21)
 s.r(55,80,1490,30,C['top'])
 s.l(55,105,1545,105)
 s.r(83,66,29,29,C['accent'],None,8)
 # l mark
 s.raw('<path d="M91 87 103 73 103 86z" fill="#FFFFFF" opacity=".95"/>')
 s.tx(126,81,'LADON',21,C['ink'],650,spacing='.4')
 s.tx(204,82,'EXPLORER',11,C['muted'],500,spacing='2')
 # tabs inside bar
 tabs=['项目设计','文档任务','FileSpace']
 for i,t in enumerate(tabs):
  x=363+i*170
  s.r(x,64,164,39, '#FFFFFF' if t==selected else C['top'],C['line'] if t==selected else None,10 if t==selected else 0)
  s.ic(['folder','check-square','layers' if False else 'git'][i],x+15,75,17,C['accentDark'] if t==selected else C['muted'])
  s.tx(x+39,84,t,14,C['ink'] if t==selected else C['muted'],550 if t==selected else 400)
  if i<2:s.ic('more',x+140,78,12,C['subtle'])
 s.ic('plus',883,74,20)
 # tenant
 s.r(1172,63,219,40,'#FFFFFF',C['line'],12)
 s.circ(1193,83,12,C['accentLight'])
 s.tx(1193,83,'L',13,C['accentDark'],700,'middle')
 s.tx(1213,83,'LADON 株式会社',13,C['ink'],500)
 s.ic('chevron-down',1366,75,16)
 s.ic('settings',1412,73,20)
 s.l(1457,65,1457,94)
 s.l(1485,85,1496,85,C['ink'],1.5)
 s.r(1510,76,12,12,'none',C['ink'],2,1.3)
 # navigation
 s.r(55,106,1490,71,'#FFFFFF')
 s.l(55,176,1544,176)
 s.ic('arrow-left',88,125,21)
 s.ic('arrow-right',128,125,21,C['subtle'])
 s.ic('arrow-up',169,125,20)
 s.l(207,119,207,164)
 s.r(229,118,776,46,C['sidebar'],C['line'],11)
 s.ic('folder',245,131,18,C['accentDark'])
 crumbs=['此电脑','工作 (D:)','Projects','LADON','项目设计']
 xx=276
 for i,t in enumerate(crumbs):
  s.tx(xx,141,t,14,C['ink'] if i==4 else C['muted'],500 if i==4 else 400)
  xx+= [72,95,91,75,85][i]
  if i<4:s.ic('chevron-right',xx-12,133,16,C['subtle'])
 s.r(1027,118,407,46,'#FFFFFF',C['line'],11)
 s.ic('search',1044,130,20)
 s.tx(1077,141,'在 项目设计 中搜索',14,C['subtle'])
 s.ic('sparkles',1398,130,20,C['accent'])
 s.ic('more',1480,131,21)
 # sidebar
 s.r(55,177,250,719,C['sidebar'])
 s.l(304,177,304,896)
 s.tx(86,215,'工作空间',12,C['subtle'],550,spacing='1')
 nav=[('home','主页'),('clock','最近使用'),('folder','所有文件'),('check-square','文档任务'),('git','FileSpace'),('cloud','云端存储')]
 for i,(ic,label) in enumerate(nav):
  y=244+i*57
  on=label==active
  if on:s.r(74,y-3,209,47,C['selected'],None,11)
  s.ic(ic,91,y+8,21,C['accentDark'] if on else C['muted'])
  s.tx(126,y+19,label,15,C['ink'] if on else C['muted'],600 if on else 400)
  if label=='文档任务':s.circ(272,y+19,3,C['gold'])
 s.tx(86,640,'已连接',12,C['subtle'],550,spacing='1')
 s.ic('hard-drive',91,665,20,C['muted']);s.tx(126,676,'本地磁盘 D:',14,C['muted'])
 s.ic('cloud',91,708,20,C['accent']);s.tx(126,719,'Google Drive',14,C['ink'])
 s.circ(271,719,4,C['accent'])
 s.l(79,791,278,791)
 s.circ(100,827,16,C['accentLight']);s.tx(100,827,'S',12,C['accentDark'],650,'middle')
 s.tx(124,820,'当前用户',13,C['ink'],500);s.tx(124,839,'个人偏好与安全',11,C['subtle'])
 s.ic('settings',262,815,19)
 # status bar
 s.l(55,896,1544,896)
 s.r(55,897,1490,55,'#FCFDFC',None,0)
 s.tx(82,924,'5 个项目',12,C['muted'])
 s.circ(1365,925,4,C['accent']);s.tx(1379,925,'已同步',12,C['muted'])
 s.tx(1467,925,'100%',12,C['subtle'])
 # chat always visible dock
 if chat=='slim':chatbar(s)

def chatbar(s):
 # Always the final bottom slot: y=884..949; never float halfway up the file viewport.
 s.r(314,883,1215,64,'#FFFFFF',C['line'],14,shadow='popover')
 s.r(328,893,122,43,C['light'],None,10)
 s.ic('sparkles',337,905,17,C['accentDark']);s.tx(363,915,'AI',13,C['ink'],600)
 s.ic('chevron-down',421,908,15,C['muted'])
 s.r(459,893,122,43,C['light'],None,10)
 s.ic('users',469,905,18,C['accentDark']);s.tx(497,915,'团队',13,C['ink'],600)
 s.ic('chevron-down',548,908,15,C['muted'])
 s.l(592,896,592,936)
 s.tx(614,915,'询问 AI 或给成员发送消息…',14,C['subtle'])
 s.ic('paperclip',1406,906,19,C['muted'])
 s.r(1451,892,62,46,C['accentDark'],None,11)
 s.ic('send',1471,904,22,'#FFFFFF')
 # toggle chat history, anchored to toolbar not as a separate floating app.
 s.r(280,892,30,45,C['accentLight'],None,10)
 s.ic('chevron-up',285,905,20,C['accentDark'])

def header(s,heading,sub=None,actions=None,top=202):
 s.tx(338,top+17,heading,22,C['ink'],600)
 if sub:s.tx(338,top+46,sub,13,C['muted'])
 if actions:
  for i,x in enumerate(actions):
   s.btn(1489-(i+1)*154,top+4,x,w=140)

def files(s,right=True,offset=0,chat=False):
 xx=338; end=right and 1175 or 1495
 s.tx(339,224,'名称',13,C['muted'],500)
 s.tx(782,224,'类型',13,C['muted'],500)
 s.tx(970,224,'最近修改',13,C['muted'],500)
 if not right:s.tx(1300,224,'同步',13,C['muted'],500)
 s.l(336,245,end-15,245)
 rows=[('file','要件定义.md','Markdown','今天 10:24','已同步'),('file','基本设计.md','Markdown','今天 09:18','已同步'),('file','详细设计.md','Markdown','昨天 16:42','已同步'),('file','测试设计.xlsx','Excel','11月16日 14:08','已同步'),('git','architecture.nema','.NEMA','11月15日 11:20','已同步')]
 for i,(ico,name,typ,date,st) in enumerate(rows):
  y=253+i*68
  if i==0:s.r(331,y, end-345,61,C['selected'],None,10)
  if ico=='git':s.ic('git',350,y+21,22,C['accent'])
  else:s.ic('file',351,y+21,23,C['accentDark'] if i==0 else C['muted'])
  s.tx(391,y+32,name,16,C['ink'],600 if i==0 else 400)
  if i==0:s.pill(530,y+20,'任务 · 3',C['softGold'],C['gold'],11,w=83)
  s.tx(782,y+32,typ,13,C['muted'])
  s.tx(970,y+32,date,13,C['muted'])
  if not right:
   s.ic('circle-check',1300,y+20,20,C['accent']);s.tx(1330,y+33,st,12,C['muted'])
  s.l(336,y+67,end-15,y+67)
 if right:
  s.l(1188,199,1188,816)
  s.tx(1221,224,'文件详情',12,C['subtle'],500,spacing='1')
  s.ic('file',1221,266,26,C['accentDark']);s.tx(1262,280,'要件定义.md',16,C['ink'],600)
  s.l(1216,317,1495,317)
  det=[('版本','v12'),('事件','24 条'),('关联任务','3 项'),('云端','已归档')]
  for i,(k,v) in enumerate(det):
   yy=347+i*43
   s.tx(1221,yy,k,13,C['muted']);s.tx(1481,yy,v,13,C['ink'],500,'end')
  s.l(1216,526,1495,526)
  s.tx(1221,560,'最近变更',12,C['subtle'])
  s.circ(1228,604,4,C['accent']);s.tx(1248,604,'REQ-023 条款更新',13,C['ink'])
  s.tx(1248,627,'今天 10:24 · Agent KAI',11,C['muted'])
  s.btn(1218,687,'查看完整历史','history',w=263)

def chat_panel(s,team=False):
 s.r(315,501,1214,446,'#FFFFFF',C['line'],16,shadow='popover')
 s.r(315,501,1214,53,C['top'],None,14)
 s.ic('message',342,517,19,C['accentDark'])
 s.tx(369,531,'沟通记录',15,C['ink'],600)
 s.r(453,508,104,36,C['accentLight'] if not team else '#FFFFFF',None,9)
 s.tx(505,528,'AI 助手',13,C['ink'] if not team else C['muted'],550,'middle')
 s.r(564,508,111,36,C['accentLight'] if team else '#FFFFFF',None,9)
 s.tx(620,528,'团队 (3)',13,C['ink'] if team else C['muted'],550,'middle')
 s.tx(1331,530,'隐藏记录',12,C['muted']);s.ic('chevron-down',1411,518,18)
 s.ic('more',1486,518,20)
 s.l(315,554,1528,554)
 if not team:
  s.circ(359,602,16,C['accentLight']);s.tx(359,602,'AI',11,C['accentDark'],700,'middle')
  s.tx(390,594,'LADON Assistant',13,C['ink'],600);s.tx(536,594,'10:31',11,C['subtle'])
  s.tx(390,621,'已检测到要件定义中 3 项变更，关联 2 份设计文档和 4 个测试点。',14,C['ink'])
  s.r(390,640,688,39,C['light'],None,9);s.ic('link',406,652,17,C['accentDark']);s.tx(431,660,'REQ-023 · 点击查看变更影响与任务',12,C['accentDark'],500)
  s.circ(359,729,16,C['blueLight']);s.tx(359,729,'S',12,C['blue'],650,'middle')
  s.tx(390,721,'你',13,C['ink'],600);s.tx(420,721,'10:33',11,C['subtle'])
  s.tx(390,750,'生成任务计划，但暂时不要修改文件。',14,C['ink'])
 else:
  s.circ(359,602,16,C['blueLight']);s.tx(359,602,'K',12,C['blue'],650,'middle')
  s.tx(390,594,'Kato',13,C['ink'],600);s.tx(441,594,'10:26',11,C['subtle'])
  s.tx(390,622,'REQ-023 的验收条件已更新，请确认 ST-014。',14,C['ink'])
  s.circ(359,729,16,C['softGold']);s.tx(359,729,'M',12,C['gold'],650,'middle')
  s.tx(390,721,'Mika',13,C['ink'],600);s.tx(446,721,'10:29',11,C['subtle'])
  s.tx(390,748,'已补充批注，等差异报告完成后一起确认。',14,C['ink'])
 # permanent, bottom-docked input shared by expanded and collapsed view
 s.r(330,882,1183,62,C['light'],C['line'],13)
 s.r(342,892,112,41,'#FFFFFF',C['line'],9)
 s.ic('sparkles',354,903,18,C['accentDark']);s.tx(380,914,'AI' if not team else '团队',14,C['ink'],550)
 s.tx(470,914,'发送消息，@提及成员或 Agent，附带文件和条款…',14,C['subtle'])
 s.ic('paperclip',1398,902,20)
 s.r(1448,891,57,43,C['accentDark'],None,10);s.ic('send',1465,900,23,'#FFFFFF')

def context_menu(s,old=False):
 x,y=630,230; w=330; h=620
 s.r(x,y,w,h,'#FFFFFF',C['line'],13,shadow='popover')
 if old:
  s.tx(x+20,y+30,'Windows Shell 原生扩展',12,C['muted'],550)
  s.l(x+18,y+50,x+w-18,y+50)
  rows=[('folder','打开','Enter'),('external','打开方式','›'),('copy','复制','Ctrl+C'),('pencil','重命名','F2'),('archive','7-Zip','›'),('git','TortoiseGit','›'),('git','Git Bash Here',''),('shield','Windows Defender 扫描',''),('share','共享','›'),('trash','删除','Del'),('info','属性','Alt+Enter')]
  for i,(ic,txt,ky) in enumerate(rows):
   yy=y+57+i*41
   if i in (4,5):s.r(x+9,yy-4,w-18,38,C['light'],None,7)
   s.ic(ic,x+18,yy+4,18,C['muted']);s.tx(x+54,yy+15,txt,14,C['ink']);s.tx(x+w-18,yy+15,ky,11,C['subtle'],anchor='end')
  s.l(x+15,y+513,x+w-15,y+513)
  s.ic('shield',x+20,y+528,19,C['accentDark']);s.tx(x+55,y+541,'添加为敏感文件',14,C['ink'],600)
  s.tx(x+55,y+565,'写入 .LADONignore',11,C['subtle'])
 else:
  rows=[('folder','打开','Enter'),('external','打开方式','›'),('copy','复制','Ctrl+C'),('pencil','重命名','F2'),('trash','删除','Del'),('archive','压缩为 ZIP','›')]
  for i,(ic,txt,ky) in enumerate(rows):
   yy=y+12+i*41
   s.ic(ic,x+19,yy+8,18,C['muted']);s.tx(x+54,yy+18,txt,14,C['ink']);s.tx(x+w-19,yy+18,ky,11,C['subtle'],anchor='end')
  s.l(x+17,y+266,x+w-18,y+266)
  rows=[('history','查看完整历史'),('message','文档即任务'),('sparkles','AI 批量标签'),('shield','添加为敏感文件')]
  for i,(ic,txt) in enumerate(rows):
   yy=y+282+i*45
   if i==3:s.r(x+10,yy-4,w-20,45,C['selected'],None,8)
   s.ic(ic,x+20,yy+7,18,C['accentDark']);s.tx(x+54,yy+17,txt,14,C['ink'],600 if i==3 else 400)
  s.tx(x+55,y+475,'加入 .LADONignore  ·  禁止远端外传',11,C['muted'])
  s.l(x+15,y+495,x+w-15,y+495)
  s.ic('more',x+20,y+519,18);s.tx(x+54,y+530,'显示更多 Windows 经典菜单',13,C['ink'])
  s.tx(x+w-20,y+530,'›',12,C['subtle'],anchor='end')

def rightpane_title(s,heading,small=None):
 s.tx(338,232,heading,23,C['ink'],650)
 if small:s.tx(338,268,small,13,C['muted'])
 s.l(335,295,1494,295)

def document_scene(s):
 rightpane_title(s,'要件定义.md','产品设计  /  要件定义  /  版本 v12')
 s.pill(1328,219,'已批准要件',C['accentLight'],C['accentDark'],12,w=127)
 s.r(335,314,847,484,'#FFFFFF',C['line'],11)
 s.tx(371,355,'要件定义书',24,C['ink'],650)
 s.tx(371,396,'REQ-023  日本侧检品复测流程',16,C['ink'],600)
 s.l(370,416,1144,416)
 s.tx(370,456,'系统应允许针对已关联制造批次的检品数据执行复测。',15,C['ink'])
 s.tx(370,488,'原始记录不可覆盖，复测结果必须保留独立轮次。',15,C['ink'])
 s.r(368,515,779,98,C['selected'],None,10)
 s.r(367,515,4,98,C['accent'],None,2)
 s.tx(389,540,'批注 · 文档即任务',12,C['accentDark'],600)
 s.tx(389,575,'请追加“日本侧复测时差异对比”的验收条件。',14,C['ink'])
 s.tx(389,598,'Kato  ·  昨天 16:42  ·  待审批',11,C['muted'])
 s.tx(370,651,'关联资源',13,C['muted'],500)
 for i,t in enumerate(['基本设计.md','ST-014','architecture.nema']):
  s.pill(371+i*153,672,t,C['light'],C['muted'],12,w=144)
 s.tx(370,753,'✎  双击段落编辑  ·  选中文本添加批注',12,C['subtle'])
 s.r(1204,314,291,484,C['sidebar'],None,11)
 s.tx(1228,349,'关联任务',15,C['ink'],600)
 s.pill(1226,369,'3 待处理',C['softGold'],C['gold'],11,w=86)
 for i,(a,b) in enumerate([('检查基本设计','FileSpace A'),('补充详细设计','FileSpace B'),('更新 ST-014','FileSpace C')]):
  y=429+i*88;s.ic('check-square',1229,y,18,C['accent']);s.tx(1260,y+11,a,13,C['ink'],500);s.tx(1260,y+36,b+' · 可并行',11,C['muted']);s.l(1227,y+62,1475,y+62)
 s.btn(1224,741,'生成影响分析','sparkles',w=250)

def history_scene(s):
 rightpane_title(s,'要件定义.md  /  修改历史','完整可追溯时间线 · Turso 索引 + 云端历史对象 · 可恢复')
 s.r(335,315,737,473,'#FFFFFF',C['line'],11)
 s.tx(365,353,'v12  ·  当前版本',17,C['ink'],600)
 s.pill(852,336,'云端已校验',C['accentLight'],C['accentDark'],12,w=164)
 s.l(365,384,1044,384)
 for i,(tag,d,who,info) in enumerate([('v12','今天 10:24','Agent KAI','REQ-023 验收条件更新'),('v11','11月17日 16:42','Kato','增加复测流程描述'),('v10','11月16日 10:05','你','调整测试对应关系'),('v09','11月15日 17:20','Agent M','规范化文档章节')]):
  y=410+i*86
  s.circ(381,y+13,8,C['accentLight'] if i==0 else '#FFFFFF',C['accent'] if i==0 else C['line'])
  if i<3:s.l(381,y+22,381,y+88,C['line'],2)
  s.tx(403,y+10,tag+'   '+info,15,C['ink'],600 if i==0 else 400)
  s.tx(403,y+35,who+'  ·  '+d,12,C['muted'])
  s.pill(920,y-5,'查看 Diff',C['light'],C['accentDark'],11,w=112)
 s.r(1095,315,399,473,C['sidebar'],None,11)
 s.tx(1122,353,'本次差异  ·  v11 → v12',16,C['ink'],600)
 s.r(1123,382,342,115,C['riskLight'],None,8)
 s.tx(1139,412,'−  原流程允许复测。',13,C['risk'])
 s.r(1123,510,342,115,C['accentLight'],None,8)
 s.tx(1139,539,'+  复测保留独立轮次。',13,C['accentDark'])
 s.tx(1139,571,'+  差异对比默认仅看变化。',13,C['accentDark'])
 s.tx(1123,655,'事件 ID   EV-00018429',12,C['muted'])
 s.tx(1123,683,'归档对象   GDRIVE / CAS-8AF1',12,C['muted'])
 s.btn(1122,720,'生成恢复计划','history',w=345)

def tagging_scene(s):
 files(s,right=False)
 # overlay shade to show modal focus
 s.r(305,192,1239,694,'#213025',None,0,opacity='.16')
 s.r(553,268,573,530,'#FFFFFF',C['line'],17,shadow='popover')
 s.ic('sparkles',587,299,22,C['accent']);s.tx(625,311,'AI 批量标签',21,C['ink'],650)
 s.tx(588,347,'当前文件夹 · 5 个文件  /  可包含子文件夹',13,C['muted'])
 s.l(585,371,1094,371)
 s.tx(586,407,'标签维度',14,C['ink'],600)
 for i,tag in enumerate(['项目归属','IPA 阶段','文档用途','关联要件','状态']):
  x=585+(i%3)*170;y=429+(i//3)*49
  s.r(x,y,159,36,C['selected'] if i!=4 else '#FFFFFF',C['line'],9)
  s.ic('circle-check' if i!=4 else 'plus',x+12,y+9,17,C['accentDark'] if i!=4 else C['subtle'])
  s.tx(x+37,y+20,tag,13,C['ink'])
 s.tx(588,553,'自定义分类规则',14,C['ink'],600)
 s.r(586,575,504,88,C['sidebar'],C['line'],10)
 s.tx(602,600,'按日本 IPA 要件、基本、详细、UT、IT、ST 阶段分类；',13,C['ink'])
 s.tx(602,625,'保留与已批准要件的关联，增量更新。',13,C['ink'])
 s.ic('shield',591,686,18,C['accent']);s.tx(617,697,'遵守 .LADONignore · 仅处理允许访问的内容',12,C['muted'])
 s.btn(772,733,'取消',w=125)
 s.btn(911,733,'开始分析','sparkles',w=179,primary=True)

def tenant_scene(s):
 files(s,right=True)
 s.r(1138,115,345,537,'#FFFFFF',C['line'],14,shadow='popover')
 s.tx(1161,148,'切换工作空间',17,C['ink'],600)
 s.tx(1161,174,'当前账号：sora@ladon.example',12,C['muted'])
 s.l(1159,196,1459,196)
 arr=[('L','LADON 株式会社','管理员','3 名成员'),('P','个人空间','所有者','仅自己'),('R','Research Team','编辑者','8 名成员')]
 for i,(letter,n,role,participants) in enumerate(arr):
  y=207+i*83
  if i==0:s.r(1149,y,325,74,C['selected'],None,9)
  s.circ(1180,y+36,17,C['accentLight'] if i==0 else C['light'])
  s.tx(1180,y+36,letter,13,C['accentDark'],650,'middle')
  s.tx(1209,y+24,n,14,C['ink'],600)
  s.tx(1209,y+49,role+' · '+participants,12,C['muted'])
  if i==0:s.ic('check',1430,y+25,21,C['accent'])
 s.l(1158,472,1465,472)
 s.ic('users',1166,488,20);s.tx(1196,499,'成员与角色管理',14,C['ink'])
 s.ic('cloud',1166,538,20);s.tx(1196,549,'跨终端同步设置',14,C['ink'])
 s.ic('shield',1166,588,20);s.tx(1196,599,'租户数据与访问策略',14,C['ink'])

def storage_scene(s):
 rightpane_title(s,'存储中心','工作空间存储设置 · 本地优先 · 云端归档可配置')
 cards=[(335,'本地热数据','2.4 GB','主要文件、Turso 索引与缓存','hard-drive'),(727,'Google Drive','128.4 GB','历史版本、加密归档、事件段','cloud'),(1119,'NAS · 未连接','—','SMB / WebDAV 适配器已预留','folder')]
 for x,title,value,desc,ico in cards:
  s.r(x,319,371,172,C['sidebar'],C['line'],12)
  s.ic(ico,x+25,343,22,C['accentDark']);s.tx(x+58,355,title,16,C['ink'],600)
  s.tx(x+26,408,value,29,C['ink'],650)
  s.tx(x+26,454,desc,12,C['muted'])
 s.r(335,513,1155,268,'#FFFFFF',C['line'],12)
 s.tx(362,550,'历史归档队列',18,C['ink'],600)
 s.pill(1320,532,'同步正常',C['accentLight'],C['accentDark'],12,w=145)
 items=[('要件定义.md · v12','完整历史 · CAS','已验证','circle-check'),('基本设计.md · v08','增量事件段 · 46 KB','已验证','circle-check'),('ST-014.xlsx · v05','等待上传 · 128 KB','待处理','clock')]
 for i,(a,b,status,ic) in enumerate(items):
  y=573+i*67;s.ic('file',362,y+8,22,C['muted']);s.tx(398,y+15,a,14,C['ink'],500);s.tx(398,y+38,b,11,C['muted']);s.ic(ic,1313,y+9,18,C['accent'] if i!=2 else C['gold']);s.tx(1344,y+21,status,12,C['muted']);s.l(360,y+60,1467,y+60)

def agent_scene(s):
 rightpane_title(s,'FileSpace · 并行工作区','TaskSpace 模式 · Snapshot / FileRef / Delta · 统一 Commit Coordinator')
 xlist=[(336,'A','要件影响分析','运行中','REQ-023 → 基本设计','53%'),(728,'B','测试追踪更新','等待审批','ST-014 / IT-006','85%'),(1120,'C','AI 标签索引','已完成','5 个文件 · IPA 分类','100%')]
 for x,id,name,status,sub,pct in xlist:
  s.r(x,325,370,197,'#FFFFFF',C['line'],12)
  s.circ(x+36,363,17,C['accentLight']);s.tx(x+36,363,id,15,C['accentDark'],650,'middle')
  s.tx(x+67,356,name,16,C['ink'],600)
  s.tx(x+67,382,sub,12,C['muted'])
  s.pill(x+25,421,status,C['softGold'] if status=='等待审批' else C['accentLight'],C['gold'] if status=='等待审批' else C['accentDark'],11,w=95)
  s.r(x+25,477,318,5,C['light'],None,2)
  s.r(x+25,477,int(318*int(pct[:-1])/100),5,C['accent'],None,2)
 s.r(336,544,1154,236,C['sidebar'],C['line'],12)
 s.tx(360,578,'Agent 操作计划  ·  提交前审查',17,C['ink'],600)
 for i,(t,why,tag) in enumerate([('读取 REQ-023 变更记录','FileRef · 只读 · 可并行','已完成'),('更新基本设计.md','基于已批准版本 v12','待审批'),('验证 ST-014 测试对应','检查版本冲突后提交','排队中')]):
  y=597+i*55
  s.ic('circle-check' if i==0 else 'clock',360,y+12,19,C['accent'] if i==0 else C['gold'])
  s.tx(390,y+20,t,14,C['ink'],500)
  s.tx(680,y+20,why,12,C['muted'])
  s.pill(1323,y+4,tag,C['light'],C['muted'],11,w=113)


def ignore_scene(s):
 rightpane_title(s,'.LADONignore · 敏感资源保护','兼容 gitignore 风格规则 · 本地生效 · 跨云盘统一执行')
 s.r(335,318,777,477,'#FFFFFF',C['line'],11)
 s.r(335,318,777,44,C['sidebar'],None,11)
 s.tx(360,339,'.LADONignore',13,C['ink'],600,family=FMONO)
 s.ic('shield',1064,329,18,C['accent'])
 code=['# 项目敏感资源排除','# 凭据与密钥','.env','.env.*','**/secrets/','**/*.pem','','# 私人文件','private/','**/*_confidential.*']
 for i,t in enumerate(code):
  y=394+i*34;s.tx(362,y,str(i+1).zfill(2),12,C['subtle'],family=FMONO)
  s.tx(408,y,t,15,C['accentDark'] if t.startswith('#') else C['ink'],family=FMONO)
 s.r(1135,318,358,477,C['sidebar'],None,11)
 s.tx(1160,354,'规则检测',16,C['ink'],600)
 s.tx(1160,395,'测试文件路径',12,C['muted'])
 s.r(1161,411,307,44,'#FFFFFF',C['line'],8)
 s.tx(1172,434,'private/client-data.xlsx',13,C['ink'],family=FMONO)
 s.r(1160,477,308,66,C['riskLight'],None,9)
 s.ic('shield',1173,495,19,C['risk']);s.tx(1203,504,'已排除云端归档',14,C['risk'],600)
 s.tx(1174,566,'本地历史保留',13,C['ink']);s.ic('circle-check',1435,555,19,C['accent'])
 s.tx(1174,608,'远程 AI / Embedding',13,C['ink']);s.tx(1459,608,'禁止',12,C['risk'],500,'end')
 s.tx(1174,649,'外部 MCP / 插件',13,C['ink']);s.tx(1459,649,'禁止',12,C['risk'],500,'end')
 s.btn(1159,721,'查看影响文件','eye',w=305)


def shell_menu_scene(s):
 files(s,right=True)
 context_menu(s,old=True)
 # secondary explanatory card top-right small
 s.r(978,308,337,87,C['sidebar'],C['line'],11)
 s.ic('info',997,324,18,C['accentDark']);s.tx(1023,334,'系统原生扩展菜单',14,C['ink'],600)
 s.tx(999,364,'兼容安装的第三方 Shell 命令',12,C['muted'])

def save(name,render,selected='项目设计',active='所有文件',chat='slim',base_files=False,right=True):
 s=SVG(name)
 base(s,selected=selected,active=active,chat=chat)
 if base_files:files(s,right=right)
 render(s)
 data=s.finish();filename=f'{len(list(OUT.glob("*.svg")))+1:02d}_{name}.svg';(OUT/filename).write_text(data,encoding='utf-8');return OUT/filename

def hover_scene(s):
 files(s,right=False)
 # collapse sidebar into icon rail, only reveal submenu on hover
 s.r(55,178,250,718,C['window'],None,0)
 s.r(55,178,77,718,C['sidebar'])
 for i,name in enumerate(['home','folder','check-square','git','cloud','settings']):
  yy=213+i*80
  s.ic(name,80,yy,27,C['accentDark'] if i==2 else C['muted'])
 s.r(142,369,252,185,'#FFFFFF',C['line'],11,shadow='popover')
 s.tx(166,398,'文档任务',15,C['ink'],600)
 for i,(ic,name) in enumerate([('file','任务总览'),('message','条款批注'),('history','审批历史')]):
  s.ic(ic,169,421+i*43,18,C['muted']);s.tx(198,432+i*43,name,13,C['ink'])
 s.r(1213,263,250,45,C['light'],None,9)
 s.ic('copy',1234,276,18);s.ic('history',1290,276,18);s.ic('more',1347,276,18)
 s.r(610,314,348,83,'#FFFFFF',C['line'],10,shadow='popover')
 s.ic('sparkles',629,333,20,C['accentDark']);s.tx(661,345,'已建立 AI 标签',14,C['ink'],600)
 s.tx(636,376,'悬停查看来源与规则版本',12,C['muted'])

def folder_api_scene(s):
 files(s,right=False)
 s.r(1024,266,466,525,'#FFFFFF',C['line'],14,shadow='popover')
 s.ic('git',1044,294,20,C['accentDark']);s.tx(1076,306,'Agent 文件夹 API',18,C['ink'],600)
 s.tx(1054,344,'FileRef · Policy · Scope · Audit',12,C['muted'])
 s.l(1048,371,1462,371)
 for i,(ic,name,desc) in enumerate([('folder','浏览目录','分页 / Tree / FileRef'),('file','安全读取','版本指纹 / 敏感规则'),('search','精确检索','Grep / 标签查询'),('history','完整历史','版本 / Diff / 恢复'),('tag','批量标签','IPA 阶段 / 自定义规则'),('message','事件订阅','游标 / ACK / Delta'),('shield','提交审批','Agent 写入计划 / CAS')]):
  yy=387+i*53
  s.ic(ic,1058,yy+8,21,C['accentDark']);s.tx(1093,yy+20,name,14,C['ink'],600);s.tx(1271,yy+20,desc,11,C['muted'])
 s.tx(1058,777,'仅已授权 Agent 可调用 · HTTP/MCP 服务待实现',11,C['risk'])

items=[]
items.append(save('极简默认浏览',lambda s:files(s,right=True),base_files=False))
items.append(save('AI聊天展开',lambda s:(files(s,right=False),chat_panel(s)),base_files=False))
items.append(save('成员聊天展开',lambda s:(files(s,right=False),chat_panel(s,team=True)),base_files=False))
items.append(save('右键扩展操作',lambda s:(files(s,right=True),context_menu(s)),base_files=False))
items.append(save('Explorer传统菜单',shell_menu_scene))
items.append(save('文档即任务编辑',document_scene,selected='文档任务',active='文档任务'))
items.append(save('完整版本历史',history_scene,selected='文档任务',active='文档任务'))
items.append(save('AI批量打标签',tagging_scene))
items.append(save('账号与租户切换',tenant_scene))
items.append(save('Google云盘与NAS',storage_scene,active='云端存储'))
items.append(save('多Agent并行工作区',agent_scene,selected='FileSpace',active='FileSpace'))
items.append(save('LADON敏感文件排除',ignore_scene))
items.append(save('悬停与侧栏折叠',hover_scene))
items.append(save('Agent文件夹API',folder_api_scene,selected='FileSpace',active='FileSpace'))

# Fix text where foreground icon was a fake layers placeholder? not issue.
# render pngs
for p in items:
 png=p.with_suffix('.png')
 cairosvg.svg2png(url=str(p),write_to=str(png),output_width=1280,output_height=800)

# contact sheet 3 cols with each screenshot at 480x300, titled
thumbw,thumbh=480,300
margin,gutter,head=34,26,45
cols=3;rows=math.ceil(len(items)/cols)
sheet=Image.new('RGB',(margin*2+cols*thumbw+(cols-1)*gutter,margin*2+rows*(thumbh+head+20)),C['bg'])
d=ImageDraw.Draw(sheet)
fontpath='/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc'
try:
 f=ImageFont.truetype(fontpath,19)
 fs=ImageFont.truetype(fontpath,13)
except Exception:
 f=ImageFont.load_default();fs=f
for i,p in enumerate(items):
 x=margin+(i%cols)*(thumbw+gutter); y=margin+(i//cols)*(thumbh+head+20)
 png=Image.open(p.with_suffix('.png')).convert('RGB').resize((thumbw,thumbh),Image.Resampling.LANCZOS)
 d.rounded_rectangle((x-2,y-2,x+thumbw+2,y+thumbh+2),radius=10,fill='#FFFFFF')
 sheet.paste(png,(x,y))
 d.text((x+6,y+thumbh+9),f'{i+1:02d}  {p.stem.split("_",1)[-1]}',font=f,fill=C['ink'])

sheet_path=OUT/'00_新版14场景总览.png'
sheet.save(sheet_path,optimize=True)

# HTML offline gallery with SVGs as img
html=['<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>LADON Explorer · 现代侘寂 · 14场景</title><style>*{box-sizing:border-box}body{margin:0;background:#e9ede7;color:#27312c;font-family:Inter,system-ui,"Noto Sans CJK SC",sans-serif}.wrap{max-width:1520px;margin:0 auto;padding:48px 28px}h1{font-size:26px;font-weight:650;letter-spacing:-.5px}p{color:#738078;font-size:14px}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(460px,1fr));gap:25px;margin-top:36px}.card{background:#fff;border:1px solid #dfe7df;border-radius:16px;overflow:hidden;box-shadow:0 8px 22px #30463110}.card img{width:100%;display:block}.meta{padding:16px 18px;display:flex;justify-content:space-between;align-items:center}.meta a{color:#4e684f;text-decoration:none;font-size:13px;font-weight:600}.foot{font-size:12px;color:#778178;margin-top:35px}</style></head><body><div class="wrap"><h1>LADON Explorer <span style="font-weight:400;color:#718570">— Modern Wabi-Sabi</span></h1><p>轻盈原生桌面风格｜主界面最小化｜功能渐进呈现｜12 个可编辑 SVG 设计状态</p><div class="grid">']
for i,p in enumerate(items):
 html.append(f'<div class="card"><img src="{escape(p.name)}" alt="{escape(p.stem)}"><div class="meta"><b>{i+1:02d} · {escape(p.stem.split("_",1)[-1])}</b><a href="{escape(p.name)}">查看 SVG ↗</a></div></div>')
html.append('</div><div class="foot">设计原型，不代表已实现功能。右键菜单为交互视觉示意，实际 Windows Shell 兼容需独立开发及系统测试。</div></div></body></html>')
(OUT/'index.html').write_text('\n'.join(html),encoding='utf-8')
(OUT/'README.md').write_text('''# LADON Explorer · Modern Wabi-Sabi · UI state prototypes\n\n设计原则：原生 Explorer 式简洁、无仿古纹理、留白、暖白与苔绿、功能按需显现。\n\n## 场景\n\n'''+ '\n'.join(f'- {p.stem}' for p in items)+'''\n\n## 使用\n\n- 在浏览器打开 `index.html` 查看所有 SVG。\n- SVG 均为独立可编辑纯矢量文件，尺寸 1600×1000。\n- 对应 PNG 仅为预览。\n- 默认底部常驻一行 AI / 成员通信栏；聊天记录可以按需展开或隐藏。\n- Explorer 右键以 Windows Shell 兼容为实现目标；当前只是 UI 示意。\n- 交互、数据库、云同步、租户权限及 Agent 功能均需按正式设计书开发。\n''',encoding='utf-8')

zip_path=OUT.parent/'LADON_UI_V1_2_独立原型.zip'
with zipfile.ZipFile(zip_path,'w',zipfile.ZIP_DEFLATED,compresslevel=8) as z:
 for p in sorted(OUT.rglob('*')):
  if p.is_file():z.write(p,'LADON_Explorer_UI/'+str(p.relative_to(OUT)))
 z.write(sheet_path,'LADON_Explorer_UI/00_14场景总览.png')

print('SVG_COUNT',len(items))
print('ZIP',zip_path,zip_path.stat().st_size)
print('SHEET',sheet_path,sheet_path.stat().st_size)
print('SAMPLE',items[0])
