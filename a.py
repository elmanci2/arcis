
import json
for v in ["warm","soft","light"]:
    p=f"/home/nuxa/.vscode/extensions/littensy.charmed-icons-0.10.0/dist/themes/{v}/theme.json"
    d=json.load(open(p))
    d["iconDefinitions"]["arcis"]={"iconPath":"./icons/arcis.svg"}
    d["fileExtensions"]["tsr"]="arcis"
    d["languageIds"]["arcis"]="arcis"
    json.dump(d,open(p,"w"))
    print(f"✓ {v}")
  