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

local port = "48228"

local endpoint = "http://" .. ipaddr .. ":" .. port

local idfile = fs.open("id", "r")

local id = nil
local command = nil
local backoff = 0;

if not idfile then
    local fuel = turtle.getFuelLevel()
    if fs.exists("/disk/pos") then
        io.input("/disk/pos")
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
    if command == "Wait" then
        sleep(5)
    elseif command == "Forward" then
        turtle.forward()
    elseif command == "Backward" then
        turtle.back()
    elseif command == "Left" then
        turtle.turnLeft()
    elseif command == "Right" then
        turtle.turnRight()
    elseif command == "Up" then
        turtle.up()
    elseif command == "Down" then
        turtle.down()
    elseif command == "Update" then
        args = {...}
        if args[1] == "nested" then
            -- no exec = stack overflow
            break
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
        below = below
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
        print("C&C server offline, waiting " .. backoff .. " seconds")
        sleep(backoff)
        backoff = backoff + 1
    end
until command == "Poweroff"
