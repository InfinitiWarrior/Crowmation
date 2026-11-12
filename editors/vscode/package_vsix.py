#!/usr/bin/env python3
"""Build crowmation.vsix without extra tools. A .vsix is a zip with a manifest.

    npm install && python3 package_vsix.py && code --install-extension crowmation.vsix
"""
import json
import os
import zipfile

here = os.path.dirname(os.path.abspath(__file__))
pkg = json.load(open(os.path.join(here, "package.json")))

manifest = f"""<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="{pkg['name']}" Version="{pkg['version']}" Publisher="{pkg['publisher']}" />
    <DisplayName>{pkg['displayName']}</DisplayName>
    <Description xml:space="preserve">{pkg['description']}</Description>
    <Categories>Programming Languages</Categories>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="{pkg['engines']['vscode']}" />
    </Properties>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code" />
  </Installation>
  <Dependencies />
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
  </Assets>
</PackageManifest>
"""

files = ["package.json", "extension.js", "language-configuration.json", "syntaxes/cwm.tmLanguage.json"]
for root, _, names in os.walk(os.path.join(here, "node_modules")):
    files += [os.path.relpath(os.path.join(root, n), here) for n in names]

exts = sorted({os.path.splitext(f)[1] for f in files if os.path.splitext(f)[1]} | {".vsixmanifest"})
types = "".join(f'<Default Extension="{e}" ContentType="application/octet-stream"/>' for e in exts)
content_types = f'<?xml version="1.0" encoding="utf-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">{types}</Types>'

out = os.path.join(here, "crowmation.vsix")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", content_types)
    z.writestr("extension.vsixmanifest", manifest)
    for f in files:
        z.write(os.path.join(here, f), "extension/" + f)
print(f"wrote {out} ({len(files)} files)")
