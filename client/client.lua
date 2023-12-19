local function refuel()
    turtle.select(16)
    turtle.dropUp()
    while turtle.getFuelLevel() ~= turtle.getFuelLimit() do
        turtle.suck()
        turtle.refuel()
    end
end

local function dump()
    for i = 1, 16, 1 do
        turtle.select(i)
        turtle.drop()
    end
end

local port = "48228"
local endpoint = "http://" .. ipaddr .. ":" .. port

local function update(args)
    if args[1] == "nested" then
        -- no exec = stack overflow
        return false
    end
    local req = http.get(endpoint .. "/turtle/client.lua")
    if not req then
        os.reboot()
    end
    local update = req.readAll()
    req.close()
    fs.delete("startup-backup")
    if fs.exists("/startup") then
        -- pcall does not work with cc fs
        fs.move("startup", "startup-backup")
    end
    local startup = fs.open("startup", "w")
    startup.write(update)
    startup.close()
    shell.run("startup", "nested")
    return true
end

local function cycle(func, n)
    for i = 1, n, 1 do
        if not func() then
            return false
        end
    end
    return true
end

if not ipaddr then
    if fs.exists("/disk/ip") then
        local ipfile = fs.open("/disk/ip")
        ipaddr = ipfile.readAll()
        ipfile.close()
    else
        print("enter server ip:")
        ipaddr = read("l")
    end
end

local idfile = fs.open("id", "r")

local id = nil
local command = nil
local backoff = 0;

if not idfile then
    local fuel = turtle.getFuelLevel()
    if fs.exists("/disk/pos") then
        io.input("/disk/pos")
    else
        io.input(io.stdin)
    end
    local startpos = io.input()
    print("Direction (North, South, East, West):")
    local direction = startpos:read("l")
    print("X:")
    local x = tonumber(startpos:read("l"))
    print("Y:")
    local y = tonumber(startpos:read("l"))
    print("Z:")
    local z = tonumber(startpos:read("l"))

    local info = {
        fuel = fuel,
        position = {x, y, z},
        facing = direction,
    }
    -- TODO: get from boot floppy
    local turtleinfo = http.post(
        endpoint .. "/turtle/new",
        textutils.serializeJSON(info),
        { ["Content-Type"] = "application/json" }
    )
    local response = textutils.unserialiseJSON(turtleinfo.readAll())

    idfile = fs.open("id", "w")
    idfile.write(response.id)
    idfile.close()
    os.setComputerLabel(response.name)
    id = response.id
    command = response.command
else
    id = idfile.readAll()
    idfile.close()
end

repeat
    print(command)
    local args = nil
    if type(command) == "table" then
        command, args = pairs(command)(command)
    end

    local ret = nil

    if command == "Wait" then
        sleep(args)
    elseif command == "Forward" then
        ret = cycle(turtle.forward, args)
    elseif command == "Backward" then
        ret = cycle(turtle.back, args)
    elseif command == "Left" then
        ret = turtle.turnLeft()
    elseif command == "Right" then
        ret = turtle.turnRight()
    elseif command == "Up" then
        ret = cycle(turtle.up, args)
    elseif command == "Down" then
        ret = cycle(turtle.down, args)
    elseif command == "Dig" then
        ret = turtle.dig()
    elseif command == "DigUp" then
        ret = turtle.digUp()
    elseif command == "DigDown" then
        ret = turtle.digDown()
    elseif command == "ItemInfo" then
        ret = { Item = turtle.getItemDetail(args) }
    elseif command == "Refuel" then
        refuel()
    elseif command == "Dump" then
        dump()
    elseif command == "Update" then
        if not update({...}) then
            break
        end
    end

    command = nil

    local ret_table = nil
    if type(ret) == "boolean" then
        if ret then
            ret_table = "Success"
        else
            ret_table = "Failure"
        end
    else
        ret_table = ret
    end

    if not ret_table then
        ret_table = "None"
    end

    local ahead = "minecraft:air"
    local above = "minecraft:air"
    local below = "minecraft:air"

    local a,b = turtle.inspect()
    if a then
        ahead = b.name
    end

    local a,b = turtle.inspectUp()
    if a then
        above = b.name
    end

    local a,b = turtle.inspectDown()
    if a then
        below = b.name
    end
    local info = {
        fuel = turtle.getFuelLevel(),
        ahead = ahead,
        above = above,
        below = below,
        ret = ret_table,
    }
    print(info.ret)

    local rsp = http.post(
        endpoint .. "/turtle/" .. id  .. "/update" ,
        textutils.serializeJSON(info),
        { ["Content-Type"] = "application/json" }
    )
    if rsp then
        backoff = 0
        command = textutils.unserialiseJSON(rsp.readAll())
    else
        print("C&C server offline, waiting " .. backoff .. " seconds")
        sleep(backoff)
        backoff = backoff + 1
    end
until command == "Poweroff"
