-- Installed by GMod Manager when addon syncing is enabled. Sandbox normally leaves
-- weapon/entity tiles empty when an addon omits its class-named icon.
local function findMaterial(class)
  local function try(name)
    if file.Exists("materials/entities/" .. name .. ".png", "GAME") then
      local material = Material("entities/" .. name .. ".png")
      if material and not material:IsError() then return material, "entities/" .. name .. ".png" end
    end
    if file.Exists("materials/entities/" .. name .. ".vmt", "GAME") then
      local material = Material("entities/" .. name)
      if material and not material:IsError() then return material, "entities/" .. name end
    end
  end

  -- Variants often reuse their parent's artwork (e.g. arccw_ammo_smg1_more).
  -- Some addons prefix the class but keep their art under the short name.
  local stems = { class, class:match("^[^_]+_(.+)$") }
  for _, stem in ipairs(stems) do
    for _ = 1, 3 do
      stem = stem:match("^(.*)_[^_]+$")
      if not stem then break end
      local material, path = try(stem)
      if material then return material, path end
    end
  end
end

local function modelFor(kind, class)
  if kind == "weapon" then
    local weapon = weapons.GetStored(class)
    return weapon and weapon.WorldModel
  end
  local entity = scripted_ents.GetStored(class)
  return entity and entity.t and entity.t.Model
end

local function install()
  if not vgui or not vgui.GetControlTable then return end
  local panel = vgui.GetControlTable("ContentIcon")
  if not panel or panel.GMMOriginalSetMaterial then return end
  panel.GMMOriginalSetMaterial = panel.SetMaterial
  function panel:SetMaterial(name)
    self:GMMOriginalSetMaterial(name)
    local kind = self:GetContentType()
    if kind ~= "weapon" and kind ~= "entity" then return end
    local current = self.Image:GetMaterial()
    if current and not current:IsError() then return end

    local class = self:GetSpawnName()
    if not class or class == "" then return end
    local material, path = findMaterial(class)
    if material then
      self.Image:SetMaterial(material)
      self.m_MaterialName = path
      return
    end

    local model = modelFor(kind, class)
    if type(model) == "string" and model:sub(1, 7):lower() == "models/"
        and file.Exists(model, "GAME") then
      local preview = vgui.Create("SpawnIcon", self)
      preview:SetMouseInputEnabled(false)
      preview:SetKeyboardInputEnabled(false)
      preview:SetPos(8, 5)
      preview:SetSize(112, 92)
      preview:SetModel(model)
      return
    end

    -- Never leave an unrecognizable gray tile when the addon supplied no usable art or model.
    self:GMMOriginalSetMaterial(kind == "weapon" and "icon16/gun.png" or "icon16/bricks.png")
  end
end

hook.Add("InitPostEntity", "GMMSpawnIconFallback", install)
timer.Simple(0, install)
