local endpoint = "http://" .. ipaddr .. ":" 
if port then
    endpoint = endpoint .. port
else
    endpoint = endpoint .. "48228"
end

local args = {...}
local function update()
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

local function cyclefn(fn)
    return function (n)
        for i = 1, n, 1 do
            if not fn() then
                return false
            end
        end
    end
end

local function iteminfo(slot)
    return { ["Item"] = turtle.getItemDetail(slot) }
end

local function inventoryinfo()
    return { ["Inventory"] = peripheral.wrap("front").list() }
end

local function drawui(command, args, backoff)
    term.setTextColor(colors.white)
    local name = os.getComputerLabel()
    local width, height = term.getSize()
    local offset = math.floor((width - string.len(name)) / 2)
    term.setCursorPos(offset, 2)
    term.write(name)

    term.setCursorPos(1,4)
    term.clearLine()
    term.write("C&C: ")
    if backoff == 0 then
        term.setTextColor(colors.green)
        term.write("connected")
    else
        term.setTextColor(colors.red)
        term.write("disconnected ")
        term.setTextColor(colors.gray)
        term.write(backoff+1 .. " failed attempts")
        
    end

    term.setCursorPos(1,6)
    term.setTextColor(colors.white)
    term.clearLine()
    if command then
        term.write("Command: ")
        term.write(command .. " ")
        term.setTextColor(colors.blue)
        term.write(args)
    end

    term.setTextColor(colors.white)
    term.setCursorPos(1,8)
    term.clearLine()
    term.write("fuel: ")
    term.setTextColor(colors.blue)
    term.write(string.format("%06d" ,turtle.getFuelLevel()))
    term.setCursorPos(7,9)
    term.write(string.format("%06d" ,turtle.getFuelLimit()))
    term.setCursorPos(1,10)
end

local commands = {
    ["Wait"] = sleep,
    ["Forward"] = cyclefn(turtle.forward),
    ["Backward"] = cyclefn(turtle.back),
    ["Up"] = cyclefn(turtle.up),
    ["Down"] = cyclefn(turtle.down),
    ["DropFront"] = turtle.drop,
    ["DropUp"] = turtle.dropUp,
    ["DropDown"] = turtle.dropDown,
    ["SuckFront"] = turtle.suck,
    ["SuckUp"] = turtle.suckUp,
    ["SuckDown"] = turtle.suckDown,
    ["Select"] = turtle.select,
    ["Refuel"] = turtle.refuel,
    ["ItemInfo"] = iteminfo,
    ["InventoryInfo"] = inventoryinfo,
    ["Left"] = turtle.turnLeft,
    ["Right"] = turtle.turnRight,
    ["Dig"] = turtle.dig,
    ["DigUp"] = turtle.digUp,
    ["DigDown"] = turtle.digDown,
    ["PlaceUp"] = turtle.placeUp,
    ["Place"] = turtle.place,
    ["PlaceDown"] = turtle.placeDown,
    ["Update"] = update,
    ["Poweroff"] = os.shutdown,
    ["GetFuelLimit"] = turtle.getFuelLimit,
};

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
    local maxfuel = turtle.getFuelLimit()
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
        fuellimit = maxfuel,
        position = {x, y, z},
        facing = direction,
    }
    ::request::
    local turtleinfo = http.post(
        endpoint .. "/turtle/new",
        textutils.serializeJSON(info),
        { ["Content-Type"] = "application/json" }
    )
    if not turtleinfo then
        print("server not responding")
        sleep(1)
        goto request
    end
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

term.clear()

repeat
    local args = nil
    if type(command) == "table" then
        command, args = pairs(command)(command)
    end

    drawui(command, args, backoff)

    local ret = nil

    if command then
        ret = commands[command](args)
    end

    if command == "Update" and ret == false then
        break
    end

    command = nil

    local ret_table = nil
    if type(ret) == "boolean" then
        if ret then
            ret_table = "Success"
        else
            ret_table = "Failure"
        end
    elseif ret then
        ret_table = ret
    else
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

    local rsp = http.post(
        endpoint .. "/turtle/" .. id  .. "/update" ,
        textutils.serializeJSON(info),
        { ["Content-Type"] = "application/json" }
    )
    if rsp then
        backoff = 0
        command = textutils.unserialiseJSON(rsp.readAll())
    else
        sleep(backoff)
        backoff = backoff + 1
    end
until command == "Poweroff"
